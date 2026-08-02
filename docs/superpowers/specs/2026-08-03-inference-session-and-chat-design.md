# Inference session and chat — design

**Status:** Approved, not yet implemented.
**Date:** 2026-08-03
**Covers:** ROADMAP M4.3 (inference session) and M4.4 (chat), taken together.
**Does not cover:** M4.2 (model acquisition). See [Scope](#scope).

## 1. Context

ADR-008 built an advisor that says whether a model fits. ADR-012 chose how to
run one — fetch a prebuilt `llama-server` and execute it as a subprocess — and
`osstat-inference` implements the acquisition half. Its own module doc names
the gap: "the first half of letting you act on that."

This design is the second half. It spawns the runtime that is already on disk,
against a model file the user already has, and puts a chat interface in front
of it with the token figures visible.

The state it builds on, verified rather than assumed:

- `llama-server` acquisition works. A CUDA runtime is installed on the
  development machine at `runtimes/b10194/win-cuda-13.3-x64`.
- Nothing in the codebase spawns it. There is no session, no HTTP client
  against it, and no chat UI.
- Nothing acquires a model. The registry in `osstat-llm` holds parameter counts
  and source notes for arithmetic; it deliberately holds no download URLs, and
  there is no GGUF file on the development machine.

### Scope

The user's request — "run the selected LLM, chat with it, see token usage,
production grade" — spans three roadmap milestones. They are built in this
order, each with its own spec, plan and branch:

1. **This spec (M4.3 + M4.4):** session, chat, token usage. The model is one
   the user points osstat at on disk.
2. **Next (M4.2):** Hugging Face search and resumable GGUF download, landing
   into a chat that already works.

The roadmap lists M4.2 first. This design inverts that deliberately. Model
acquisition is the largest and most network-dependent of the three, and at the
end of it the user still could not chat — the same "no visible payoff of its
own" problem M4.1 identified about itself. Building the session first proves
the hard parts (process supervision, streaming, teardown) against a fixed local
file, before any download UI exists to confuse a failure with.

"The selected LLM" therefore means, in this spec, **the model file the user
opened**. Section 5 defines how that relates to the advisor's fit matrix.

## 2. Decisions

These were settled during brainstorming and are inputs to the design, not
questions it reopens.

| Question             | Decision                                                                                           |
| -------------------- | -------------------------------------------------------------------------------------------------- |
| Process lifetime     | Alive only while the chat page is open; torn down on leave.                                        |
| Conversation history | Persisted to disk, with a visible delete.                                                          |
| Token figures shown  | Context fill meter, live tokens/sec, per-message prompt/completion counts. **Not** session totals. |
| Model identity       | The file's GGUF header is the truth; the registry annotates it.                                    |
| Streaming path       | Rust owns the socket; the webview never issues an HTTP request.                                    |
| Output rendering     | Plain text with fenced code blocks detected. No markdown dependency.                               |

### Why the webview never speaks HTTP

ADR-012's consequences state a security property: "The CSP is untouched. All
egress is in Rust; the webview never issues an HTTP request, so a compromised
webview still cannot exfiltrate (threat 3)."

Letting the front end fetch `127.0.0.1:<port>` directly would be markedly less
code — native `EventSource`, no event protocol, no Rust HTTP client — and would
retire that property. It would also put the session API key in the webview.
This design keeps the property. The cost is an SSE parser and an event protocol
in Rust, and that cost is accepted explicitly.

### Why the process dies when you leave the page

A loaded 7B model at Q4 holds roughly 5 GB resident. osstat's stated budgets are
an installer under ~20 MB, cold start under 2 s and idle RAM under 150 MB. A
monitoring utility that quietly holds 5 GB while sitting in the tray has become
the thing it measures. The cost — a few seconds of model load on returning to
the page — is paid visibly, in a place the user asked to be.

## 3. Architecture

A new crate, **`osstat-chat`**, rather than growing `osstat-inference`.

`osstat-inference` acquires: download, verify, unpack, store. Its code is nearly
pure and its tests are hermetic. A session owns a child process and a long-lived
HTTP stream — different dependencies, different testing character. The workspace
already splits along these lines (`core` / `platform` / `llm` / `inference`).

`osstat-chat` carries `#![forbid(unsafe_code)]`, like every crate except
`osstat-platform`. Anything needing FFI moves to `osstat-platform` rather than
weakening this — established during the shared-GPU work, where `forbid` cannot
be overridden by a scoped `allow` and the probes were relocated instead.

### Modules

| Module                  | Responsibility                                           | Depends on                            |
| ----------------------- | -------------------------------------------------------- | ------------------------------------- |
| `osstat-chat::gguf`     | Parse a GGUF header into a `ModelFile`. Pure.            | nothing                               |
| `osstat-chat::plan`     | Choose `-ngl` and `-c` for a file on this machine. Pure. | `osstat-llm`, `osstat-core`           |
| `osstat-chat::session`  | Spawn, supervise, stop `llama-server`.                   | `osstat-platform`, `osstat-inference` |
| `osstat-chat::client`   | OpenAI-compatible client; SSE to typed events.           | `reqwest`                             |
| `osstat-chat::store`    | Conversations on disk. Pure over a directory.            | nothing                               |
| `src-tauri/src/chat.rs` | Tauri commands and `chat:*` events.                      | `osstat-chat`                         |
| `ui/src/pages/Chat.tsx` | The page.                                                | Tauri events                          |

`src-tauri/src/chat.rs` sits beside the existing `runtime.rs` and follows its
shape. `runtime.rs` exists because "a spawned task and three events" did not
belong in the one-line-adapter file; the same reasoning applies here.

`reqwest` 0.13 with `rustls` is already a workspace dependency, used by
`osstat-inference::download`. No new HTTP stack.

### Data flow for one message

```
UI: invoke("chat_send", { conversationId, text })
  → chat.rs appends the user message, persists
  → client POSTs /v1/chat/completions { stream: true, timings_per_token: true }
  → SSE chunk arrives
      → chat:token   { conversationId, delta, timings }
      → ... repeated ...
  → final chunk carries usage
      → chat:complete { conversationId, usage, timings }
  → chat.rs persists the exchange
```

The context meter's denominator is `n_ctx`, fetched once from `/props` at
session start. Its numerator is the last exchange's
`usage.prompt_tokens + usage.completion_tokens`.

## 4. The session

### Startup sequence

Each step has a named failure reported where the user can act on it. A generic
"failed to start" is the outcome this sequence exists to prevent.

1. **No runtime installed.** Nothing spawns. The page points at the Settings
   section that acquires one.
2. **Model file unreadable, or not a GGUF.** The header parse fails before
   anything spawns — the cheapest possible failure.
3. **`plan` says it will not fit** — not even one layer on the GPU, or the
   weights exceed system memory too. Warn, naming the shortfall. **Do not
   refuse.** The arithmetic is explicitly an estimate; refusing on an estimate
   would make osstat wrong in a way the user cannot override.
4. **`llama-server` exits during startup.** Its stderr is captured and shown. A
   missing CUDA DLL or a rejected argument produces a specific line there, and
   swallowing it is the difference between a fixable problem and a shrug.
5. **Started but not yet ready.** Poll `GET /health`, which returns
   `503 {"error":{"code":503,"message":"Loading model",...}}` until the model is
   loaded and then `200 {"status":"ok"}`. No fixed short timeout — a 30 GB model
   on a slow disk would trip it. The page shows loading, with a cancel.

### The argument vector

Arguments are passed as a vector, never through a shell. This is SECURITY.md's
existing control for the runtime binary, unchanged.

| Flag                 | Value              | Why                                                                                                                                                |
| -------------------- | ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--host`             | `127.0.0.1`        | Loopback only. This is also llama-server's default; set explicitly so a future upstream default change cannot widen it silently.                   |
| `--port`             | OS-allocated       | Bind `127.0.0.1:0`, read the port, release, pass it. Never 8080 — a fixed port collides with whatever else the user runs.                          |
| `--api-key`          | random per session | Nothing else on the machine can drive the model through this port. Generated per session, never written to disk, never sent to the webview.        |
| `--no-webui`         | —                  | llama-server serves its own web UI **by default**. osstat spawns a server for its own use; it does not put an unrequested web app on a local port. |
| `-ngl`               | computed by `plan` | §5.1. Bounded by the header's block count.                                                                                                         |
| `-c`                 | computed by `plan` | §5.1. Never left to upstream's default.                                                                                                            |
| `--tools`, `--agent` | **never passed**   | These enable built-in tools including `exec_shell_command` and `write_file`. Default off upstream. osstat must never turn them on.                 |

**Context size is chosen deliberately.** A model's header may declare a context
length of 128k or more; allocating that much KV cache can exhaust VRAM on a
machine where the weights themselves fit comfortably. `-c` is never left to
whatever upstream's default happens to be — that default has changed across
releases, which would make osstat's memory behaviour depend on which pinned tag
is installed. §5.1 defines how it is chosen.

### Supervision

ADR-012 chose a subprocess so that an inference OOM would not be "a crash of the
whole app — taking down the tray and the sampler with it," on the grounds that
"a monitoring tool that dies because a model was too big has failed at its
actual job."

Supervision makes good on that claim:

- The child's exit is detected whether or not a request is in flight.
- The session is marked dead; the conversation stays on screen.
- `chat:failed` carries the tail of stderr, which is where the OOM message is.
- osstat keeps sampling. The tray keeps working.

This is the property most worth guarding, and §9 gives it a test that kills the
child mid-stream.

### Cancel

Cancelling aborts the response stream; `llama-server` stops generating when its
client disconnects. **The partial text stays in the conversation, marked
stopped.** Discarding tokens the user already watched arrive would be a worse
outcome than keeping them with an honest label.

### Teardown without orphans

Normal exits are straightforward: leaving the page or quitting the app stops the
child.

The hard case is osstat being killed, leaving a child holding several GB. The
answer is **not** a Windows Job Object: that is unsafe FFI, and unsafe code
lives in `osstat-platform` or nowhere. Instead:

- The session records the child's **PID and start time** to a file under the
  app-data directory.
- On next launch, osstat reaps any recorded child still alive.
- The PID/start-time pair is checked before signalling, reusing the identity
  check `osstat-platform` already has for ADR-006.

PID reuse is the trap here, and it is the same trap ADR-006 already solved once:
a recorded PID may belong to an unrelated process by the time osstat next runs.
The start-time comparison is what makes reaping safe, and §9 tests it against a
real process rather than a mock.

## 5. Model identity

The GGUF header is the source of truth. The registry annotates it.

`gguf::parse` reads a `ModelFile`:

| Field            | From                    | Used for                              |
| ---------------- | ----------------------- | ------------------------------------- |
| `architecture`   | `general.architecture`  | Looking the model up in the registry. |
| `block_count`    | `<arch>.block_count`    | The upper bound on `-ngl`.            |
| `context_length` | `<arch>.context_length` | The cap on `-c`.                      |
| `quantization`   | `general.file_type`     | Display, and registry reconciliation. |
| `parameters`     | tensor shapes           | Display, and registry reconciliation. |

When the architecture and parameter count match a registry entry, the page shows
the advisor's verdict for it as context. When they do not, the model runs
anyway — the registry is a curated list, not a permission list, and refusing to
run a file because osstat has not heard of it would be indefensible.

### 5.1 Choosing `-ngl` and `-c` from the file

The advisor cannot supply these. `calculator::evaluate` **takes** a context
length rather than producing one, and it needs a `ModelEntry` — so a model the
registry has never seen would have no verdict and therefore no layer count. A
design that read `-ngl` off the fit matrix would work only for the fifteen
models in the registry, and silently not for anything else.

The calculator's building blocks are public and take exactly what the header
supplies, so `plan` computes both figures from the file itself:

| `ModelArchitecture` field | GGUF header key                                                   |
| ------------------------- | ----------------------------------------------------------------- |
| `num_layers`              | `<arch>.block_count`                                              |
| `hidden_size`             | `<arch>.embedding_length`                                         |
| `num_attention_heads`     | `<arch>.attention.head_count`                                     |
| `num_kv_heads`            | `<arch>.attention.head_count_kv`                                  |
| `head_dim`                | `<arch>.attention.key_length`, or `embedding_length / head_count` |
| `max_context_length`      | `<arch>.context_length`                                           |

With that, and the **actual file size on disk** rather than a bits-per-weight
estimate:

1. `select_gpu_budget(&devices)` gives the VRAM budget from the live probe.
2. `-c` starts at the smaller of the header's `context_length` and a default
   ceiling, then halves while `kv_cache_bytes(-c, arch)` alone exceeds the
   budget. A context that cannot fit its own KV cache is not a context.
3. `-ngl` is the largest layer count whose
   `per_layer_weight_bytes(file_size, num_layers)` share, plus the KV cache,
   fits the remaining budget — capped at `block_count`.

This is what makes "the file is the truth" operational rather than a slogan:
the figures come from measured file size and declared architecture, on this
machine's probed hardware. A registry match adds the advisor's verdict as
context and changes nothing about what is passed to `llama-server`.

Both steps are pure functions over `(ModelFile, file_size, GpuBudget)` and are
table-tested, including the degenerate cases: no GPU at all (`-ngl 0`), a budget
smaller than one layer, and a header claiming zero layers.

**A partial parse is a failure, not a result.** A half-read header would yield a
confident `-ngl` for the wrong model. This is the rule `parse_luid_instance`
already follows on the GPU side, for the same reason: a figure that is real and
attributed wrongly is worse than no figure.

## 6. Persistence

One JSON file per conversation under the app-data directory. Not a database.

- Inspectable: the user can read what osstat kept, which matches its
  auditability pitch.
- Deletable: delete is removing a file, and the UI control does exactly that.
- No schema migration, no embedded engine, no new dependency.

`store` is pure over a directory path, so its tests use `tempfile` — the same
approach `RuntimeStore` already takes.

**SECURITY.md gains an entry.** osstat now stores what the user typed. Where it
is stored, that it is not encrypted, that deleting a conversation removes the
file, and what uninstalling does — all written down rather than discovered.
This is a real change to what osstat is, and the sixth threat entry is part of
this work, not a follow-up.

## 7. The UI

`ui/src/pages/Chat.tsx`. Top to bottom: model bar, transcript, composer.

**Model bar** — the session's identity and controls: model name from the header,
backend, the `-ngl` chosen, a stop control, and the advisor's verdict when the
model is recognised. A dead session announces itself here, where the user is
already looking.

**Context fill** is a meter, reusing `ui/src/components/Meter.tsx` — the same
component the Overview draws CPU and RAM with. That reuse is the point: this is
a monitoring application, and the context window is the resource governing
whether the model still remembers the start of the conversation. It sits in the
model bar and fills as the conversation grows.

**Live tokens/sec** appears only while generating, beside the streaming message,
showing both figures `timings` provides: prompt processing speed
(`prompt_per_second`) and generation speed (`predicted_per_second`). Both
matter and they differ — a long context spends seconds in prompt eval before the
first token appears, and a user watching nothing happen deserves to see why.
This is also the figure that closes the loop back to the advisor: it says
whether the `-ngl` chosen was right.

**Per-message counts** sit under each exchange, muted: prompt tokens in,
completion tokens out. Enough to notice a system prompt eating the window.

**No session totals** — declined, and derivable from the per-message figures.

**A system prompt field**, collapsed by default.

**No sampling controls.** Temperature and top-p serve a use case not described,
and llama.cpp's defaults are reasonable. Easy to add later against a chat that
works; not justified now.

**Rendering** is plain text with whitespace preserved, plus fenced code blocks
detected and rendered monospaced with a copy button. No markdown dependency, and
**no HTML is ever constructed from model output** — so there is no sanitiser to
get wrong. The UI's runtime dependency list stays at four packages.

The visible cost, stated plainly: a model that formats heavily will show
literal `**` and `|`. Accepted.

## 8. Error taxonomy

Every failure reaches the UI as a typed variant with a message naming what to do
about it. None of these may surface as a panic, and none may leave a truncated
message presented as complete.

| Variant           | Cause                                       | What the UI says                         |
| ----------------- | ------------------------------------------- | ---------------------------------------- |
| `NoRuntime`       | No `llama-server` installed                 | Points at Settings.                      |
| `ModelUnreadable` | File missing or not openable                | Names the path.                          |
| `NotAGguf`        | Bad magic, truncated, or overrunning header | Names the file, not the offset.          |
| `SpawnFailed`     | Process would not start                     | The stderr tail.                         |
| `NeverReady`      | Cancelled while loading                     | Confirms nothing is left running.        |
| `ServerDied`      | Child exited unexpectedly                   | The stderr tail; conversation preserved. |
| `StreamBroken`    | Socket closed mid-message                   | Partial text kept, marked incomplete.    |
| `BadChunk`        | Malformed SSE payload                       | Partial text kept, marked incomplete.    |
| `Cancelled`       | User stopped generation                     | Partial text kept, marked stopped.       |

## 9. Testing

Hermetic. ADR-012 already settled the principle: "CI does not download a runtime
or run inference. Both are slow and network-flaky."

**`gguf`** — fixture headers built byte by byte: valid, truncated, bad magic, and
one whose declared KV count overruns the buffer. Each malformed case must yield
an error, never a partial parse.

**`plan`** — table-tested over `(ModelFile, file_size, GpuBudget)`, including no
GPU at all, a budget smaller than one layer, a header declaring zero layers, and
a context whose KV cache alone exceeds the budget. Pure, so the whole table is
provable from any one platform — the property ADR-008 already relies on for
`select`.

**`client`** — the existing hand-rolled `TcpListener` fixture-server pattern from
`download.rs`, extended to SSE with real chunked writes and flushes. Its comment
gives the reason to keep it: "a mock would replace the very thing that could be
wrong." Cases: a stream stopping mid-message, a chunk that is not valid JSON, an
error object where a delta was expected, and a `usage` block that never arrives.

**`session`** — tested against a **stub server**, a small test binary speaking
`/health`, `/props` and a canned SSE stream, which can be told to die mid-
generation.

> **The load-bearing test:** kill the child while tokens are streaming, and
> assert osstat is still running, the sampler still sampling, and the error
> carrying the stderr tail. Without it, ADR-012's central claim is a claim
> rather than a guarantee.

**Orphan reaping** — a real short-lived process: record its PID and start time,
let it exit, and assert osstat does **not** signal whatever inherited the PID.
This needs a real process because PID reuse is exactly what a mock papers over.

**UI** — Vitest with mocked Tauri events, following `Llm.test.tsx` and
`Settings.runtime.test.tsx`. Worth writing: tokens rendering as they arrive, the
context meter moving with usage, tokens/sec appearing during generation and
stopping after, and a dead session surfacing rather than hanging. Each is proven
by deliberate breakage before it counts — the standard the shared-GPU work
arrived at after two tests were found to pass against a swapped prop.

**Real inference is a manual gate.** The platforms actually verified are named
in the report, never implied by a green build.

## 10. Consequences

- **osstat now runs a local HTTP server as a child process.** Bound to loopback,
  on an ephemeral port, behind a per-session API key, with its own web UI
  disabled.
- **osstat now stores what the user typed.** SECURITY.md gains a sixth threat
  entry covering location, lack of encryption, deletion and uninstall.
- **The CSP is untouched**, and ADR-012's "the webview never issues an HTTP
  request" remains true.
- **A new crate** brings the workspace to six.
- **No new front-end dependency.** The UI stays at four runtime packages.
- **An ADR is warranted** for the session — process supervision, the lockdown
  argument vector, and orphan reaping are decisions a later reader will need the
  reasoning for. It extends ADR-012 rather than replacing it.

## 11. Known risks

Stated now so they are triaged rather than discovered.

- **`-c` and `-ngl` come from an estimate.** `plan` uses the real file size and
  the declared architecture, which is better than the fit matrix manages, but
  ADR-008's formula is still an approximation and llama.cpp's real allocation
  includes compute buffers this does not model. A wrong figure shows up as an
  OOM at load. The mitigation is that the OOM is survivable and its stderr is
  shown — not that the estimate is right.
- **`head_dim` may need deriving.** Not every architecture writes
  `attention.key_length`; the fallback is `embedding_length / head_count`, which
  is correct for standard attention and wrong for models that diverge from it.
  A wrong `head_dim` mis-sizes the KV cache term. `plan` records which route it
  took so a bad figure is diagnosable rather than mysterious.
- **Only Windows can be verified by hand here.** The development machine is
  Windows with an NVIDIA card. The Linux and macOS session paths will type-check
  and be stub-tested, but a real spawn on those platforms is unverified until
  someone runs it — the same gap the shared-GPU work carried for its Linux path,
  and it should be reported the same way rather than quietly ticked.
- **Model files are large and slow to load.** Every manual test costs minutes.
  This makes the stub server load-bearing for iteration speed, not just for CI.
- **`llama-server`'s API is pinned only by convention.** `runtimes.json` pins a
  tag, so the flags and response shapes this design reads are those of `b10194`.
  Refreshing the pin can change them. The flags used here are stable across
  recent releases, but `--no-webui` and `timings_per_token` are newer than most,
  and a pin refresh should re-check them.
