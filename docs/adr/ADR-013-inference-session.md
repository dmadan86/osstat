# ADR-013: Inference session and chat

**Status:** Accepted

## Context

ADR-012 chose to fetch a prebuilt `llama-server` and execute it as a
subprocess, and `osstat-inference` implements that acquisition. Its own module
doc names what remained: "the first half of letting you act on that."

This decision covers the second half — running the server, talking to it, and
showing what a conversation costs. It extends ADR-012 rather than replacing it.

## Decision

**Run the server locked down, not at its defaults.** The argument vector is
`--host 127.0.0.1`, a port the OS allocated, `--api-key` generated per session,
`--no-webui`, plus `-ngl` and `-c` computed for this machine. Arguments are
passed as a vector, never through a shell.

**Never pass `--tools` or `--agent`.** They enable built-in tools including
`exec_shell_command` and `write_file`.

**Compute `-ngl` and `-c` from the model file**, not from the advisor's fit
matrix.

**The webview issues no HTTP request.** Rust owns the socket, consumes the
server-sent-event stream, and forwards `chat:*` events to the front end.

**Reap orphans with a recorded `ProcessKey`**, not a Windows Job Object.

**The process lives only while the chat page is open**, but the page does not
own the session — Rust does. It is opened by the Run control before the chat
page exists, and it survives that page being unmounted and mounted again. So the
page asks `chat_status` what is open on mount rather than assuming, and the close
that leaving performs is deferred long enough for a remount to call it off. A
close fired straight from the unmount ended the server on every remount, and the
page that came back showed a model bar over nothing.

**Conversations are persisted**, one JSON file each, with a delete control.

## Rationale

**Why the web UI is disabled.** `llama-server` serves its own chat application
by default. osstat spawns a server for its own use; putting an unrequested web
app on a local port is not something a monitoring utility should do silently.

**Why a per-session key.** Binding a port makes the model reachable by anything
else running as the same user. The key is not a secret against an attacker who
can read this process's memory — it stops other software from stumbling onto an
open inference endpoint, which is the actual risk. It is seeded from
`RandomState`, which the standard library seeds from the OS. Reading the clock
instead would have been worse than it looks: several reads in a tight loop
differ only in their last digits, so the key would be largely predictable from
the moment the session started.

**Why usage figures come from the response body.** `llama-server` returns
`usage` and `timings` on the completion itself, and `timings_per_token: true`
makes the speeds arrive during generation rather than only at the end. No second
API is needed, and — importantly — no per-process metric is involved. This is
the same trap ADR-008 recorded for `IDXGIAdapter3::QueryVideoMemoryInfo`, whose
`CurrentUsage` describes the calling application rather than the machine.

**Why `-ngl` and `-c` are computed from the file.** `calculator::evaluate` takes
a context length rather than producing one, and it needs a `ModelEntry`. A model
the registry has never heard of therefore has no verdict and no layer count.
Reading `-ngl` off the fit matrix would have worked for the fifteen models in
the registry and silently not for anything else — the same shape of gap ADR-008
carried when NVML was the only measured source and every AMD and Intel card
reported no video memory at all.

Instead the arithmetic runs over the file: its real size on disk, and the
architecture its own header declares, against the VRAM the ADR-008 probe
measured. Every field of `ModelArchitecture` has a corresponding GGUF header
key, so this needs no estimate where a measurement exists. The calculator's
terms are reused rather than reimplemented, so there is one formula in the
codebase and not two.

`-c` is never left to upstream's default. That default has changed across
releases, which would make osstat's memory behaviour depend on which pinned tag
happens to be installed. Models increasingly declare 128k contexts, and
allocating that much KV cache can exhaust VRAM on a machine where the weights
themselves fit comfortably.

**Why the webview never speaks HTTP.** ADR-012's consequences state it as a
property: "All egress is in Rust; the webview never issues an HTTP request, so a
compromised webview still cannot exfiltrate." Letting the front end fetch
`127.0.0.1:<port>` directly would have been markedly less code — native
`EventSource`, no event protocol, no Rust HTTP client — and would have retired
that property and put the session key in the webview. The cost of keeping it is
an SSE parser and an event protocol, and that cost is accepted deliberately.

**Why not a Windows Job Object for teardown.** `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`
is the textbook answer to "kill the child if the parent dies," and it is unsafe
FFI. Unsafe code lives in `osstat-platform` or nowhere — a rule this project
arrived at the hard way, when the shared-GPU probes were written into a crate
carrying `#![forbid(unsafe_code)]` and had to be relocated because `forbid`
cannot be overridden by a scoped `allow`.

The portable answer costs less and reuses more: record the child's PID and start
time, and on next launch ask `ProcessController::terminate` to end it. That
trait is already documented to re-read the PID's start time and refuse on a
mismatch, because ADR-006 needed exactly that guarantee for terminating a
user-selected process. PID reuse is the trap in both cases, and it is already
solved once.

**Why the process dies when you leave the page.** A loaded 7B model at Q4 holds
roughly 5 GB resident. osstat's stated budgets are an installer under ~20 MB,
cold start under 2 s and idle RAM under 150 MB. A monitoring utility that
quietly holds 5 GB while sitting in the tray has become the thing it measures.
The cost — a few seconds of model load on returning — is paid visibly, in the
place the user asked to be.

**Why conversations are files rather than a database.** The directory is meant
to be readable: a user can open it and see exactly what osstat kept. That is the
same argument ADR-012 makes for pinning a hash a person can review. Deleting a
conversation deletes a file, with nothing left behind to explain, and there is
no schema to migrate and no engine to embed.

## Consequences

- **osstat now stores what the user typed.** SECURITY.md gains an entry naming
  the location, the absence of encryption, what delete does, and what
  uninstalling leaves behind. This is a real change to what osstat is.
- **osstat now runs a local HTTP server**, bound to loopback, on an ephemeral
  port, behind a per-session key, with its own web UI disabled.
- **SECURITY.md's "no network requests except ones you explicitly trigger"
  still holds**, and is clarified: requests to `127.0.0.1` reach a process
  osstat itself started, and nothing leaves the machine.
- **The CSP is untouched.** The webview gained no origin.
- **A sixth crate**, `osstat-chat`, carrying `#![forbid(unsafe_code)]`.
- **No new front-end dependency.** Model output is rendered as plain text with
  fenced code blocks detected, so no HTML is ever constructed from it and there
  is no sanitiser to get wrong. The visible cost is that a model formatting
  heavily shows literal `**` and `|`.
- **CI still does not run inference.** The session is tested against a stub
  server that speaks `/health`, `/props` and a canned stream, and that can be
  told to die mid-generation — which is how ADR-012's central claim, that an
  inference OOM does not take the app down with it, becomes a test rather than
  an assertion.
- **The estimate can still be wrong.** `-ngl` and `-c` come from ADR-008's
  formula, which does not model llama.cpp's compute buffers. A wrong figure
  shows up as an out-of-memory at load. The mitigation is that the failure is
  survivable and its stderr is shown — not that the estimate is right.
