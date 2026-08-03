//! Starting, watching and stopping one `llama-server` process.
//!
//! The server is locked down deliberately rather than left at its defaults. It
//! binds loopback on a port the OS chose, its own web UI is disabled, and a
//! random per-session key means nothing else on the machine can drive the model
//! through that port. `--tools` and `--agent` — which enable
//! `exec_shell_command` and `write_file` — are never passed.

use crate::ChatError;
use crate::plan::LaunchPlan;
use osstat_core::{ProcessController as _, ProcessKey, TerminationMode};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Everything needed to start one server.
#[derive(Debug, Clone)]
pub struct Launch {
    /// The `llama-server` executable, from `osstat_inference::InstalledRuntime`.
    pub server: PathBuf,
    /// The model file to load.
    pub model: PathBuf,
    /// What the arithmetic in [`crate::plan`] chose.
    pub plan: LaunchPlan,
    /// Where to record the child's [`ProcessKey`] so a later run can reap it.
    ///
    /// Passed in rather than derived, because this crate has no business
    /// knowing where an application keeps its data — `src-tauri` resolves the
    /// app-data directory and hands the file down. `None` writes nothing, which
    /// is what the tests want: they own their children already.
    pub record: Option<PathBuf>,
}

/// A running server.
#[derive(Debug)]
pub struct Session {
    /// Where to reach it, e.g. `http://127.0.0.1:52413`.
    pub base: String,
    /// The key every request must carry.
    pub api_key: String,
    child: tokio::process::Child,
    stderr: Arc<Mutex<String>>,
    key: ProcessKey,
    record: Option<PathBuf>,
}

/// A port nothing is listening on.
///
/// Bind, read, release. A race with another process is possible in principle
/// and has never been worth guarding against here: the window is microseconds,
/// and the failure mode is a start-up error the user sees immediately rather
/// than anything silent.
///
/// # Errors
///
/// [`ChatError::Io`] if no port could be bound at all.
pub fn free_port() -> Result<u16, ChatError> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

/// A key for one session, never written to disk and never sent to the webview.
///
/// Seeded from `RandomState`, which the standard library seeds from the
/// operating system. Reading the clock instead — the obvious no-dependency
/// approach — would be worse than it looks: four reads in a tight loop differ
/// only in their last digits, so the "key" would be largely predictable from
/// the moment the session started.
///
/// This is not a secret against a determined local attacker with the ability
/// to read this process's memory. It stops other software on the machine from
/// stumbling onto an open inference endpoint, which is the actual risk of
/// binding a port.
#[must_use]
pub fn random_api_key() -> String {
    use std::fmt::Write as _;
    use std::hash::{BuildHasher as _, Hasher as _};

    let mut key = String::with_capacity(48);
    for counter in 0..3_u64 {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(counter);
        // `write!` into a String cannot fail; the result is discarded rather
        // than unwrapped so this stays inside the no-panic rule.
        let _ = write!(key, "{:016x}", hasher.finish());
    }
    key
}

/// Starts a server and waits for it to become healthy.
///
/// # Errors
///
/// [`ChatError::SpawnFailed`] if the process would not start or exited during
/// startup, carrying the tail of its stderr.
pub async fn start(launch: Launch) -> Result<Session, ChatError> {
    let port = free_port()?;
    let api_key = random_api_key();

    let mut command = tokio::process::Command::new(&launch.server);
    command
        // The model path is passed straight through as an argument. The test
        // stub keys its behaviour off sentinel paths (`--slow-start`,
        // `--die-after`, `--fail-to-start`) that arrive here; they mean
        // nothing to a real `llama-server` and exist only for the stub.
        .arg("-m")
        .arg(&launch.model)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--api-key")
        .arg(&api_key)
        // llama-server serves its own web UI by default. osstat spawns a
        // server for its own use; it does not put an unrequested web app on a
        // local port.
        .arg("--no-webui")
        .arg("-ngl")
        .arg(launch.plan.gpu_layers.to_string())
        .arg("-c")
        .arg(launch.plan.context_length.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        // A dropped session must not leave a server holding VRAM, and on
        // Windows this is load-bearing rather than tidy: tokio implements a
        // child's stderr with the blocking thread pool there, and dropping a
        // runtime waits for its blocking operations to finish. A read parked
        // on a live child's pipe therefore wedges the drop forever. Killing
        // the child closes the pipe, which ends the read.
        .kill_on_drop(true);

    // No shell, ever. Arguments are a vector; SECURITY.md's existing control.
    let mut child = command
        .spawn()
        .map_err(|error| ChatError::SpawnFailed(error.to_string()))?;

    let stderr = Arc::new(Mutex::new(String::new()));
    // Kept so a failure can wait for the pipe to drain rather than racing it.
    // Reading `stderr` the instant `try_wait` reports an exit returns whatever
    // the reader happened to have copied by then, which on a fast exit is
    // nothing at all — the child writes its reason and dies in the same breath.
    // Linux lost that race consistently where Windows won it, and an empty
    // `SpawnFailed("")` is precisely the shrug this design exists to prevent.
    let mut drained = None;
    if let Some(pipe) = child.stderr.take() {
        let sink = Arc::clone(&stderr);
        drained = Some(tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt as _;
            let mut lines = tokio::io::BufReader::new(pipe).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(mut held) = sink.lock() {
                    held.push_str(&line);
                    held.push('\n');
                    // Only the tail matters: an OOM message is the last thing
                    // written, and an unbounded buffer would grow all session.
                    let overflow = held.len().saturating_sub(4096);
                    if overflow > 0 {
                        held.drain(..overflow);
                    }
                }
            }
        }));
    }

    let key = ProcessKey {
        pid: child.id().unwrap_or(0),
        started_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs()),
    };

    // Recorded before the health wait rather than after it. Loading a 30 GB
    // model takes minutes, and a crash during those minutes is exactly when an
    // orphaned server is most likely and most expensive — it would sit holding
    // every byte of VRAM the weights occupy with nothing left to stop it.
    // Waiting until `start` returns would leave that whole window unrecorded.
    //
    // A failed write is not fatal. Reaping is a safety net; refusing to run a
    // model because the net could not be strung up would be the worse outcome.
    if let Some(path) = &launch.record {
        write_record(path, key);
    }

    let base = format!("http://127.0.0.1:{port}");
    if let Err(error) = wait_until_healthy(&base, &mut child, &stderr, drained).await {
        // The only way out of the wait is the child having died, so the record
        // now names a corpse. `reap` would survive it — it re-reads the start
        // time and refuses on a mismatch — but leaving the file would have the
        // next launch chase a PID that is already gone for no reason.
        if let Some(path) = &launch.record {
            let _ = std::fs::remove_file(path);
        }
        return Err(error);
    }

    Ok(Session {
        base,
        api_key,
        child,
        stderr,
        key,
        record: launch.record,
    })
}

/// Writes the child's identity where a later run will look for it.
fn write_record(path: &std::path::Path, key: ProcessKey) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string(&key) {
        let _ = std::fs::write(path, text);
    }
}

/// Polls `/health` until the model is loaded, or the child dies trying.
///
/// No fixed timeout. `/health` answers 503 "Loading model" while the weights
/// are read, and a 30 GB model on a slow disk can take minutes — a timeout
/// chosen to feel responsive would fail exactly the users with the most to
/// wait for. The exit condition is the child dying, which is a real failure.
async fn wait_until_healthy(
    base: &str,
    child: &mut tokio::process::Child,
    stderr: &Arc<Mutex<String>>,
    drained: Option<tokio::task::JoinHandle<()>>,
) -> Result<(), ChatError> {
    let http = reqwest::Client::new();

    loop {
        if let Ok(Some(_)) = child.try_wait() {
            // Wait for the reader to finish rather than hoping it has run. It
            // ends when the pipe reaches EOF, and the pipe reaches EOF because
            // the child just exited — so this returns promptly and, unlike a
            // yield or a sleep, it cannot return early. A single yield is what
            // was here before, and Linux lost that race every time while
            // Windows won it, producing `SpawnFailed("")`: an error whose whole
            // job is to carry the reason, arriving with the reason missing.
            if let Some(handle) = drained {
                let _ = handle.await;
            }

            let tail = stderr.lock().map_or_else(
                |_| String::from("(stderr unavailable)"),
                |held| held.clone(),
            );

            // Even a drained pipe can be empty — a child killed by a signal
            // writes nothing. Say which happened rather than returning an empty
            // string the UI would render as a blank error.
            return Err(ChatError::SpawnFailed(if tail.trim().is_empty() {
                String::from("the process exited without writing a reason")
            } else {
                tail
            }));
        }

        if let Ok(response) = http.get(format!("{base}/health")).send().await
            && response.status().is_success()
        {
            return Ok(());
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

impl Session {
    /// The identity of the child, for recording against a crash.
    ///
    /// The pair is what makes reaping safe: a bare PID may belong to something
    /// else entirely by the time osstat next runs.
    #[must_use]
    pub const fn record(&self) -> ProcessKey {
        self.key
    }

    /// Whatever the child last wrote to stderr.
    #[must_use]
    pub fn stderr_tail(&self) -> String {
        self.stderr
            .lock()
            .map(|held| held.clone())
            .unwrap_or_default()
    }

    /// Stops the server and waits for it to be gone.
    ///
    /// # Errors
    ///
    /// [`ChatError::Io`] if the child could not be waited on.
    pub async fn stop(mut self) -> Result<(), ChatError> {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;

        // Removed only once the child is confirmed gone. Removing it first
        // would open a window in which osstat has forgotten a process that is
        // still running — the exact state the record exists to prevent.
        if let Some(path) = &self.record {
            let _ = std::fs::remove_file(path);
        }

        Ok(())
    }
}

/// Ends a process recorded before a crash, if it is still the same one.
///
/// # Errors
///
/// Never returns an error for a PID that is gone or has been reused — both
/// mean there is nothing of ours to reap. Only a real refusal propagates.
pub fn reap(key: ProcessKey) -> Result<(), ChatError> {
    let mut terminator = osstat_platform::Terminator::new();

    // `ProcessController::terminate` is documented to re-read the PID's start
    // time and refuse on a mismatch. That check is the whole reason this is
    // safe to call against a PID recorded in a previous run, and it already
    // exists for ADR-006 -- there is no second implementation here.
    match terminator.terminate(key, TerminationMode::Forceful) {
        // Ended, or the PID now belongs to something else -- either way there
        // is nothing of ours left running, so both are success. One arm rather
        // than two because `clippy::match_same_arms` is denied.
        Ok(_) | Err(osstat_core::Error::IdentityMismatch { .. }) => Ok(()),
        Err(error) => Err(ChatError::SpawnFailed(error.to_string())),
    }
}
