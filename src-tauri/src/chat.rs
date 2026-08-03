//! IPC adapters for running a model and holding a conversation with it.
//!
//! Adapters only, per AGENTS.md: `osstat-chat` owns the child process, the
//! stream and the stored conversations; these translate its types into the
//! shapes the webview sees and its progress into events. The shape follows
//! [`crate::runtime`] exactly — event-name constants, one payload struct per
//! event, `app.emit(...)` from a task spawned onto `tauri::async_runtime`, and
//! commands that return as soon as the work is under way.
//!
//! **The per-session API key never leaves this process.** It lives in
//! [`ChatState`], is handed only to a [`ChatClient`] that stays in Rust, and
//! appears in no payload emitted to the webview and no value returned from a
//! command. That is the whole point of holding the socket here (ADR-012,
//! ADR-013): a compromised webview has no URL to reach and no key to reach it
//! with.

// Tauri resolves `AppHandle` and `State<'_, T>` by injecting them by value; a
// reference is not part of the command signature it accepts. Both are cheap
// handles, so the lint's concern does not apply. Same reason as `commands.rs`.
#![allow(clippy::needless_pass_by_value)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use osstat_chat::store::Message as StoredMessage;
use osstat_chat::{
    ChatClient, ChatError, Conversation, ConversationStore, Launch, Message, ModelFile, Role,
    Session, StreamEvent, Timings, Usage, plan_launch,
};
use osstat_core::ProcessKey;
use osstat_inference::RuntimeStore;
use osstat_llm::calculator::select_gpu_budget;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::sampler::Sampler;

/// Emitted for every piece of text the model produces.
pub const CHAT_TOKEN_EVENT: &str = "chat:token";
/// Emitted once when a reply finishes, whether it ran out or was stopped.
pub const CHAT_COMPLETE_EVENT: &str = "chat:complete";
/// Emitted once when a reply could not be finished at all.
pub const CHAT_FAILED_EVENT: &str = "chat:failed";

/// How much of a model file is read to parse its header.
///
/// A GGUF holds the weights after the header and can be 30 GB or more. Reading
/// the file whole to look at its first few kilobytes would stall the command
/// for minutes and take the machine's memory with it; the header itself is
/// orders of magnitude smaller than this bound.
const HEADER_BYTES: u64 = 1024 * 1024;

/// The file naming a running child, inside the app-data directory.
const SESSION_RECORD: &str = "session.json";

/// The open model, the conversations, and the way to stop a reply.
///
/// The [`Session`] is held here rather than handed to the front end because it
/// carries the API key and the port. Neither is ever serialised.
pub struct ChatState {
    session: Mutex<Option<Session>>,
    /// The open model's name, kept beside the session it belongs to.
    ///
    /// Needed because a conversation records which model it was held with, and
    /// [`chat_send`] has only the session to ask. Set and cleared in the same
    /// two places the session is, so the pair cannot drift apart.
    model_name: Mutex<Option<String>>,
    /// When the open session started, for the duration on its stop line.
    ///
    /// Set and cleared in the same two places the session is, for the same
    /// reason the name is: a start time left behind by a closed session would
    /// report the next one as having run for hours.
    opened_at: Mutex<Option<std::time::Instant>>,
    store: ConversationStore,
    cancel: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

/// What opening a model produced.
///
/// Deliberately five fields. The base URL, the port and the API key are all
/// absent: the webview has no use for any of them and no way to be trusted
/// with them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ModelSession {
    /// The model file's name, without its directory or extension.
    pub model_name: String,
    /// Layers offloaded to the GPU — what `-ngl` was given.
    pub gpu_layers: u32,
    /// The context window the server actually allocated.
    pub context_length: u32,
    /// Whether every layer fitted. `false` is a warning, never a refusal.
    pub fits: bool,
    /// Whether the KV-cache arithmetic rested on a derived head dimension.
    ///
    /// Travels to the UI because a derived figure is right for standard
    /// attention and wrong for models that diverge, and a mis-sized cache is
    /// otherwise a mystery rather than a diagnosis.
    pub head_dim_derived: bool,
}

/// More of a reply.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ChatToken {
    /// Which conversation this belongs to.
    pub conversation_id: String,
    /// The text to append.
    pub delta: String,
    /// The latest speeds, where the server has reported any yet.
    ///
    /// Carried on every token rather than only at the end, which is what makes
    /// a live tokens/sec readout possible at all.
    pub timings: Option<Timings>,
}

/// A reply that ended.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ChatComplete {
    /// Which conversation this belongs to.
    pub conversation_id: String,
    /// Token counts for the exchange, where the server reported them.
    pub usage: Option<Usage>,
    /// The last speeds the server reported.
    pub timings: Option<Timings>,
    /// Whether the user stopped it rather than the model finishing.
    pub stopped: bool,
}

/// A reply that could not be finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ChatFailure {
    /// Which conversation this belongs to.
    pub conversation_id: String,
    /// What went wrong, in the server's own words where it left any.
    ///
    /// An out-of-memory message is written to stderr and nowhere else, so a
    /// failure that reported only "the stream ended" would throw away the one
    /// sentence that explains why.
    pub message: String,
}

impl ChatState {
    /// Creates state rooted at `root`, the Tauri app-data directory.
    #[must_use]
    pub const fn new(root: PathBuf) -> Self {
        Self {
            session: Mutex::new(None),
            model_name: Mutex::new(None),
            opened_at: Mutex::new(None),
            store: ConversationStore::new(root),
            cancel: Mutex::new(None),
        }
    }

    /// A client for the open session, and the open model's name.
    ///
    /// The client carries the API key; this is the only way it is ever handed
    /// out, and it never leaves the Rust side.
    fn client(&self) -> Option<(ChatClient, String)> {
        let held = self.session.lock().ok()?;
        let session = held.as_ref()?;
        let name = self
            .model_name
            .lock()
            .ok()
            .and_then(|held| held.clone())
            .unwrap_or_default();

        Some((
            ChatClient::new(session.base.clone(), session.api_key.clone()),
            name,
        ))
    }

    /// Whatever the running server last wrote to stderr.
    fn stderr_tail(&self) -> String {
        self.session
            .lock()
            .ok()
            .and_then(|held| held.as_ref().map(Session::stderr_tail))
            .unwrap_or_default()
    }

    /// Takes the open session out of state, leaving none behind.
    ///
    /// The one place a session ends — both closing and opening a second model
    /// go through here — which is why the log line for a session ending lives
    /// here rather than at either caller.
    fn take_session(&self) -> Option<Session> {
        if let Ok(mut held) = self.model_name.lock() {
            *held = None;
        }
        let started = self
            .opened_at
            .lock()
            .ok()
            .and_then(|mut held| held.take())
            .map(|at| at.elapsed().as_secs());
        let session = self.session.lock().ok().and_then(|mut held| held.take());

        if session.is_some() {
            crate::log::session_stopped(started.unwrap_or(0));
        }

        session
    }
}

/// Puts the path into a parse failure that could not know it.
///
/// `gguf::parse` works over bytes and has no path to name, so it returns
/// `NotAGguf` with an empty one. Leaving it empty would show the user a
/// sentence about a file that is not identified.
fn named(error: ChatError, path: &Path) -> ChatError {
    match error {
        ChatError::NotAGguf { reason, .. } => ChatError::NotAGguf {
            file: path.to_path_buf(),
            reason,
        },
        other => other,
    }
}

/// Reads a model file's header and its size on disk.
fn read_header(path: &Path) -> Result<(ModelFile, u64), ChatError> {
    use std::io::Read as _;

    let unreadable = || ChatError::ModelUnreadable(path.to_path_buf());

    let file = std::fs::File::open(path).map_err(|_| unreadable())?;
    let file_size = file.metadata().map_err(|_| unreadable())?.len();

    let mut header = Vec::new();
    file.take(HEADER_BYTES)
        .read_to_end(&mut header)
        .map_err(|_| unreadable())?;

    let model = osstat_chat::parse(&header).map_err(|error| named(error, path))?;
    Ok((model, file_size))
}

/// What to call a model, from its filename.
fn model_name_of(path: &Path) -> String {
    path.file_stem().map_or_else(
        || path.to_string_lossy().into_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    )
}

/// What to call a conversation, from the message that started it.
fn title_of(text: &str) -> String {
    let trimmed = text.trim();
    let title: String = trimmed.chars().take(60).collect();

    if title.is_empty() {
        "New conversation".to_owned()
    } else if title.len() < trimmed.len() {
        format!("{title}…")
    } else {
        title
    }
}

/// A stored turn, in the shape the endpoint expects.
fn wire_message(message: &StoredMessage) -> Message {
    Message {
        role: match message.role {
            Role::System => "system".to_owned(),
            Role::User => "user".to_owned(),
            Role::Assistant => "assistant".to_owned(),
        },
        content: message.content.clone(),
    }
}

/// The app-data directory, or a sentence saying it could not be found.
fn app_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|error| format!("the app data directory could not be resolved: {error}"))
}

/// Opens a model, starts a server for it, and reports what it settled on.
///
/// # Errors
///
/// If the file is not a readable GGUF, the GPU probe has not finished, no
/// runtime is installed, or the server would not start.
#[tauri::command]
pub async fn chat_open_model(
    app: AppHandle,
    sampler: State<'_, Sampler>,
    state: State<'_, ChatState>,
    path: String,
) -> Result<ModelSession, String> {
    let path = PathBuf::from(path);
    // Every refusal below is logged by kind and never by `Display`: the
    // messages here name the model file, which is the user's data.
    let (model, file_size) = read_header(&path).map_err(|error| {
        crate::log::session_failed(error.kind());
        error.to_string()
    })?;

    // Same convention as `llm_advice` and `runtime_status`: nothing is decided
    // before the probe answers, because planning a launch against "no GPU" on
    // a machine that has one is a confident wrong answer.
    let devices = sampler.devices().ok_or_else(|| {
        crate::log::session_failed("probe_unfinished");
        "the GPU probe has not finished yet".to_owned()
    })?;
    let plan = plan_launch(&model, file_size, select_gpu_budget(&devices));

    let root = app_root(&app).inspect_err(|_| crate::log::session_failed("no_app_data"))?;
    let runtime = RuntimeStore::new(root.clone())
        .installed()
        .into_iter()
        // `installed` sorts by tag then artifact, so the last entry is the
        // newest release on disk. Acquiring a runtime and then having osstat
        // keep using the older one would be baffling.
        .next_back()
        .ok_or_else(|| {
            crate::log::session_failed(ChatError::NoRuntime.kind());
            ChatError::NoRuntime.to_string()
        })?;

    // Opening a second model must not leave the first server running. It would
    // hold every byte of VRAM the old weights occupy, which is the memory the
    // new model is about to need.
    if let Some(previous) = state.take_session() {
        let _ = previous.stop().await;
    }

    let session = osstat_chat::start(Launch {
        server: runtime.server_path,
        model: path.clone(),
        plan,
        record: Some(root.join(SESSION_RECORD)),
    })
    .await
    .map_err(|error| {
        crate::log::session_failed(error.kind());
        error.to_string()
    })?;

    // Asked of the server rather than assumed from the plan: it may round or
    // clamp what it was given, and a context meter whose denominator is a
    // request rather than a fact would mislead exactly when the window fills.
    // A server that will not answer is not a reason to refuse the session, so
    // the planned figure stands in.
    let client = ChatClient::new(session.base.clone(), session.api_key.clone());
    let context_length = client.context_length().await.unwrap_or(plan.context_length);

    let model_name = model_name_of(&path);
    if let Ok(mut held) = state.session.lock() {
        *held = Some(session);
    }
    if let Ok(mut held) = state.model_name.lock() {
        *held = Some(model_name.clone());
    }
    if let Ok(mut held) = state.opened_at.lock() {
        *held = Some(std::time::Instant::now());
    }

    // The two figures that explain a slow or failing session, and neither
    // names the model: how much was offloaded, and how wide the window is.
    crate::log::session_started(plan.gpu_layers, context_length);

    Ok(ModelSession {
        model_name,
        gpu_layers: plan.gpu_layers,
        context_length,
        fits: plan.fits,
        head_dim_derived: model.head_dim_derived,
    })
}

/// Sends a message and streams the reply through `chat:*` events.
///
/// Returns as soon as the work is spawned rather than when the reply finishes:
/// a long answer takes tens of seconds, and holding the IPC call open for its
/// duration would freeze the command channel the rest of the app uses. This is
/// the same arrangement [`crate::runtime::acquire_runtime`] uses.
///
/// # Errors
///
/// If no model is open, or the conversation could not be written. Failures
/// *during* generation arrive on `chat:failed`, not here.
#[tauri::command]
pub fn chat_send(
    app: AppHandle,
    state: State<'_, ChatState>,
    conversation_id: String,
    text: String,
) -> Result<(), String> {
    let (client, model_name) = state
        .client()
        .ok_or_else(|| "no model is open; open one before sending a message".to_owned())?;

    let mut conversation = state.store.load(&conversation_id).unwrap_or(Conversation {
        id: conversation_id,
        title: title_of(&text),
        model_name,
        messages: Vec::new(),
    });

    conversation.messages.push(StoredMessage {
        role: Role::User,
        content: text,
        usage: None,
        stopped: false,
    });
    // Saved before the reply is asked for, so a crash mid-generation loses the
    // answer rather than the question.
    state.store.save(&conversation).map_err(|error| {
        crate::log::conversation_save_failed(error.kind());
        error.to_string()
    })?;

    let history: Vec<Message> = conversation.messages.iter().map(wire_message).collect();

    let (sender, receiver) = tokio::sync::oneshot::channel();
    if let Ok(mut held) = state.cancel.lock() {
        // Replacing any previous sender drops it, which is how a stop control
        // left over from a finished reply stops meaning anything.
        *held = Some(sender);
    }

    let store = state.store.clone();
    tauri::async_runtime::spawn(async move {
        stream_reply(app, client, store, conversation, history, receiver).await;
    });

    Ok(())
}

/// Streams one reply to its end, saving and announcing whatever it produced.
async fn stream_reply(
    app: AppHandle,
    client: ChatClient,
    store: ConversationStore,
    mut conversation: Conversation,
    history: Vec<Message>,
    cancel: tokio::sync::oneshot::Receiver<()>,
) {
    let id = conversation.id.clone();
    let text = Arc::new(Mutex::new(String::new()));
    let figures = Arc::new(Mutex::new((None::<Usage>, None::<Timings>)));

    let outcome = {
        let text = Arc::clone(&text);
        let figures = Arc::clone(&figures);
        let emitter = app.clone();
        let id = id.clone();

        let streaming = client.stream(history, move |event| match event {
            StreamEvent::Delta(delta) => {
                if let Ok(mut held) = text.lock() {
                    held.push_str(&delta);
                }
                let timings = figures.lock().ok().and_then(|held| held.1);
                let _ = emitter.emit(
                    CHAT_TOKEN_EVENT,
                    ChatToken {
                        conversation_id: id.clone(),
                        delta,
                        timings,
                    },
                );
            }
            // With `timings_per_token` this arrives on nearly every chunk, so
            // it is a running total rather than an ending. The end is the
            // stream itself finishing, below.
            StreamEvent::Complete { usage, timings } => {
                if let Ok(mut held) = figures.lock() {
                    if usage.is_some() {
                        held.0 = usage;
                    }
                    if timings.is_some() {
                        held.1 = timings;
                    }
                }
            }
        });

        tokio::select! {
            finished = streaming => finished,
            // The receiver resolving means `chat_stop` fired, and the sender
            // being dropped means nothing can ever stop this reply -- so a
            // closed channel is not a cancellation. `Err` is ignored either
            // way only because the branch below distinguishes them.
            stopped = cancel => match stopped {
                Ok(()) => Err(ChatError::Cancelled),
                Err(_) => streaming_forever().await,
            },
        }
    };

    let content = text.lock().map(|held| held.clone()).unwrap_or_default();
    let (usage, timings) = figures.lock().map_or((None, None), |held| *held);

    match &outcome {
        Ok(()) | Err(ChatError::Cancelled) => {
            let stopped = outcome.is_err();
            conversation.messages.push(StoredMessage {
                role: Role::Assistant,
                content,
                usage,
                stopped,
            });
            save_or_report(&store, &conversation);
            let _ = app.emit(
                CHAT_COMPLETE_EVENT,
                ChatComplete {
                    conversation_id: id,
                    usage,
                    timings,
                    stopped,
                },
            );
        }
        Err(error) => {
            // Whatever arrived before the failure is kept. Losing a
            // half-written answer punishes the user for the model being too
            // big for the machine, which is not their mistake to pay for.
            if !content.is_empty() {
                conversation.messages.push(StoredMessage {
                    role: Role::Assistant,
                    content,
                    usage,
                    stopped: true,
                });
                save_or_report(&store, &conversation);
            }
            let _ = app.emit(
                CHAT_FAILED_EVENT,
                ChatFailure {
                    conversation_id: id,
                    message: failure_message(&app, error),
                },
            );
        }
    }
}

/// Saves a conversation, logging by kind if it could not be written.
///
/// The reply has already been generated by the time this runs, so there is
/// nobody left to return an error to — which is exactly why it is logged. This
/// is the one thing in the app whose failure loses something the user typed,
/// and the kind is all that is written: [`ChatError::Io`] renders the path.
fn save_or_report(store: &ConversationStore, conversation: &Conversation) {
    if let Err(error) = store.save(conversation) {
        crate::log::conversation_save_failed(error.kind());
    }
}

/// A future that never resolves, for a cancel channel that can never fire.
///
/// `select!` needs both branches to produce the same type. A dropped sender
/// means no stop can ever arrive, so the honest behaviour is to let the other
/// branch decide the outcome — which is what never resolving achieves.
async fn streaming_forever() -> Result<(), ChatError> {
    std::future::pending().await
}

/// What to tell the user about a failed reply.
///
/// The server's stderr wins where there is any: an out-of-memory message is
/// written there and nowhere else, and "the response stream ended" on its own
/// would describe the symptom while discarding the cause.
fn failure_message(app: &AppHandle, error: &ChatError) -> String {
    let tail = app
        .try_state::<ChatState>()
        .map(|state| state.stderr_tail())
        .unwrap_or_default();
    let tail = tail.trim();

    if tail.is_empty() {
        error.to_string()
    } else {
        ChatError::ServerDied(tail.to_owned()).to_string()
    }
}

/// Stops the reply currently streaming, if there is one.
///
/// The partial text is kept and marked as stopped rather than discarded.
#[tauri::command]
pub fn chat_stop(state: State<'_, ChatState>) {
    if let Ok(mut held) = state.cancel.lock()
        && let Some(sender) = held.take()
    {
        // Fails only if the streaming task has already finished, which means
        // there is nothing left to stop.
        let _ = sender.send(());
    }
}

/// Closes the open model and stops its server.
///
/// # Errors
///
/// If the child could not be waited on. Closing when nothing is open succeeds.
#[tauri::command]
pub async fn chat_close(state: State<'_, ChatState>) -> Result<(), String> {
    let session = state.take_session();

    match session {
        Some(session) => session.stop().await.map_err(|error| error.to_string()),
        None => Ok(()),
    }
}

/// Every conversation on disk.
#[tauri::command]
#[must_use]
pub fn chat_list(state: State<'_, ChatState>) -> Vec<Conversation> {
    state.store.list()
}

/// One conversation.
///
/// # Errors
///
/// If the identifier is unusable or the file cannot be read.
#[tauri::command]
pub fn chat_load(state: State<'_, ChatState>, id: String) -> Result<Conversation, String> {
    state.store.load(&id).map_err(|error| error.to_string())
}

/// Deletes one conversation.
///
/// # Errors
///
/// If the identifier is unusable or the file cannot be removed.
#[tauri::command]
pub fn chat_delete(state: State<'_, ChatState>, id: String) -> Result<(), String> {
    state.store.delete(&id).map_err(|error| error.to_string())
}

/// Ends a server left behind by a previous run.
///
/// osstat crashing must not leave a `llama-server` holding several gigabytes of
/// VRAM with nothing on the machine that knows to stop it. The recorded
/// identity is a PID *and* a start time, and `reap` refuses on a mismatch, so a
/// reused PID is never mistaken for ours.
///
/// Every failure here is silent by design: a startup that refused to run
/// because a stale record could not be read would be worse than the orphan.
pub fn reap_orphan(app: &AppHandle) {
    let Ok(root) = app_root(app) else {
        return;
    };
    let record = root.join(SESSION_RECORD);

    let Ok(text) = std::fs::read_to_string(&record) else {
        return;
    };

    if let Ok(key) = serde_json::from_str::<ProcessKey>(&text) {
        let _ = osstat_chat::reap(key);
    }

    // Removed whether or not it parsed. A record that cannot be read is a
    // record that will never reap anything, and keeping it only guarantees the
    // same failure on every future launch.
    let _ = std::fs::remove_file(&record);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn a_parse_failure_names_the_file_the_parser_could_not_know() {
        // `gguf::parse` works over bytes and returns an empty path. Shown to
        // the user unchanged, that is a sentence about no file in particular.
        let error = ChatError::NotAGguf {
            file: PathBuf::new(),
            reason: "the header is truncated",
        };

        let named = named(error, Path::new("/models/mistral.gguf"));

        assert!(
            named.to_string().contains("mistral.gguf"),
            "the path was not filled in: {named}"
        );
    }

    #[test]
    fn a_failure_that_is_not_a_parse_failure_is_left_alone() {
        let named = named(ChatError::NoRuntime, Path::new("/models/mistral.gguf"));

        assert!(matches!(named, ChatError::NoRuntime));
    }

    #[test]
    fn a_file_that_is_not_a_gguf_is_refused_with_its_own_name() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("not-a-model.gguf");
        std::fs::write(&path, b"this is not a model at all").unwrap();

        let outcome = read_header(&path);

        let message = match &outcome {
            Err(error) => error.to_string(),
            Ok(_) => String::new(),
        };
        assert!(
            message.contains("not-a-model.gguf"),
            "expected a named refusal, got {outcome:?}"
        );
    }

    #[test]
    fn only_the_first_megabyte_of_a_model_is_read() {
        // A GGUF can be 30 GB. Reading one whole to look at its header would
        // stall the command for minutes and exhaust memory doing it, so the
        // bound is the difference between usable and unusable rather than a
        // tidiness.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large.gguf");
        let oversized = vec![0_u8; usize::try_from(HEADER_BYTES).unwrap() * 3];
        std::fs::write(&path, &oversized).unwrap();

        // The read is bounded, so this refuses on the contents rather than
        // hanging: the size it reports is still the whole file's.
        let outcome = read_header(&path);

        assert!(outcome.is_err(), "a file of zeroes parsed as a model");
    }

    #[test]
    fn a_missing_model_file_is_reported_as_unreadable_rather_than_unparseable() {
        let outcome = read_header(Path::new("/definitely/not/here.gguf"));

        assert!(matches!(outcome, Err(ChatError::ModelUnreadable(_))));
    }

    #[test]
    fn a_model_is_named_by_its_file_rather_than_its_path() {
        assert_eq!(
            model_name_of(Path::new("/models/Mistral-7B-Q4_K_M.gguf")),
            "Mistral-7B-Q4_K_M"
        );
    }

    #[test]
    fn a_long_first_message_becomes_a_short_title() {
        let title = title_of(&"a".repeat(200));

        assert!(title.chars().count() <= 61, "the title is {title}");
        assert!(title.ends_with('…'), "a truncated title should say so");
    }

    #[test]
    fn an_empty_first_message_still_produces_a_title() {
        // A conversation with no name is one the user cannot find again.
        assert_eq!(title_of("   "), "New conversation");
    }

    #[test]
    fn stored_roles_reach_the_endpoint_under_the_names_it_expects() {
        // The endpoint reads `role` as a string. Any other spelling silently
        // changes who the model thinks said what.
        for (role, expected) in [
            (Role::System, "system"),
            (Role::User, "user"),
            (Role::Assistant, "assistant"),
        ] {
            let wire = wire_message(&StoredMessage {
                role,
                content: "hello".to_owned(),
                usage: None,
                stopped: false,
            });

            assert_eq!(wire.role, expected);
            assert_eq!(wire.content, "hello");
        }
    }

    #[test]
    fn the_open_model_payload_carries_no_way_to_reach_the_server() {
        // The security property in one assertion. The payload is allowed to
        // describe the model; it is not allowed to describe the socket. If a
        // base URL, a port or the API key ever appears here, the webview can
        // talk to the model directly and ADR-012's whole argument is gone.
        let json = serde_json::to_value(ModelSession {
            model_name: "mistral".to_owned(),
            gpu_layers: 32,
            context_length: 8192,
            fits: true,
            head_dim_derived: false,
        })
        .unwrap();
        let object = json.as_object().unwrap();

        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "contextLength",
                "fits",
                "gpuLayers",
                "headDimDerived",
                "modelName"
            ]
        );
    }

    #[test]
    fn every_event_payload_is_camel_case() {
        let token = serde_json::to_value(ChatToken {
            conversation_id: "c1".to_owned(),
            delta: "hi".to_owned(),
            timings: Some(Timings {
                prompt_per_second: Some(32.3),
                predicted_per_second: Some(52.9),
            }),
        })
        .unwrap();
        assert!(token.as_object().unwrap().contains_key("conversationId"));
        assert_eq!(
            token["timings"]["predictedPerSecond"],
            serde_json::json!(52.9),
            "the speed the UI reads has to survive the trip"
        );

        let complete = serde_json::to_value(ChatComplete {
            conversation_id: "c1".to_owned(),
            usage: Some(Usage {
                prompt_tokens: 44,
                completion_tokens: 48,
            }),
            timings: None,
            stopped: false,
        })
        .unwrap();
        assert_eq!(complete["usage"]["promptTokens"], serde_json::json!(44));
        assert_eq!(
            complete["usage"]["completionTokens"],
            serde_json::json!(48),
            "swapping these would still render a meter, so the test names both"
        );

        let failed = serde_json::to_value(ChatFailure {
            conversation_id: "c1".to_owned(),
            message: "out of memory".to_owned(),
        })
        .unwrap();
        assert!(failed.as_object().unwrap().contains_key("conversationId"));
    }

    #[test]
    fn closing_a_state_that_never_opened_a_model_takes_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let state = ChatState::new(directory.path().to_path_buf());

        assert!(state.take_session().is_none());
        assert!(
            state.client().is_none(),
            "a client was handed out with no session behind it"
        );
    }

    #[test]
    fn conversations_are_kept_under_the_directory_the_state_was_given() {
        let directory = tempfile::tempdir().unwrap();
        let state = ChatState::new(directory.path().to_path_buf());

        state
            .store
            .save(&Conversation {
                id: "c1".to_owned(),
                title: "About tea".to_owned(),
                model_name: "mistral".to_owned(),
                messages: Vec::new(),
            })
            .unwrap();

        assert_eq!(state.store.list().len(), 1);
        assert!(state.store.load("c1").is_ok());
        assert!(
            state.store.load("../escape").is_err(),
            "an id reached outside the conversation directory"
        );
    }
}
