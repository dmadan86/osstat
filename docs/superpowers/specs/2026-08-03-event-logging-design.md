# Event logging — design

**Status:** Approved, not yet implemented.
**Date:** 2026-08-03

## 1. Context

osstat has no logging. No `tracing`, no `log` crate, four stray `println!`s in
`src-tauri/src/lib.rs` and `main.rs`. When something fails on a user's machine
there is nothing to look at.

That gap has grown expensive. The app now spawns a child process, streams from a
local HTTP server, downloads multi-gigabyte files with resumption, and moves
libraries between volumes. Each of those fails in ways a stack trace cannot
explain, and three real defects in recent work were only found because CI
happened to run on another operating system.

## 2. The governing decision

**No user data is logged, at any level, with no toggle to enable it.**

This is stricter than the usual "redact by default" arrangement and it was
chosen deliberately. The reason is what _this_ application can see. A log that
recorded "all events" would contain process names and PIDs, open ports with
their remote addresses, cleaning paths, model paths, and — at debug level —
chat prompts and model replies.

The purpose of saving a log is to send it to someone. A design where the useful
log is also the sensitive one puts the user in the position of choosing between
getting help and disclosing their process list, their open ports and their
conversations. osstat's SECURITY.md says it has no telemetry and makes no
network request the user did not trigger; a log file that has to be handled
carefully would sit awkwardly beside that.

### What this costs

Stated plainly, because it is a real cost and not a rhetorical one:

- A failed download logs `download_failed reason=hash_mismatch` with no
  filename. If two downloads ran that session, the log does not say which.
- A GGUF that will not parse logs `gguf_rejected` with no path.
- A process that could not be terminated logs `terminate_refused
reason=permission_denied` with no process name.

Some bugs become undiagnosable from a log alone and need a reproduction. That is
the accepted trade.

## 3. Levels

Five levels exist underneath; three are offered.

| Setting            | `tracing` level | What it carries                                                                         |
| ------------------ | --------------- | --------------------------------------------------------------------------------------- |
| **Info** (default) | `info`          | Lifecycle: startup, probe results as counts, session start and stop, download outcomes. |
| **Debug**          | `debug`         | Adds per-operation detail: sample ticks, event emissions, state transitions.            |
| **Verbose**        | `trace`         | Adds everything, including per-chunk stream progress.                                   |

`warn` and `error` are **always enabled** regardless of the setting. An error
the user did not opt into is the one they most need when something has already
gone wrong.

## 4. Files

- `<app-data>/logs/osstat.YYYY-MM-DD.log`, one per day, via
  `tracing-appender`'s rolling writer rather than a hand-rolled rotator.
- **A retention cap**, oldest deleted first. osstat runs all day in the tray;
  unbounded logs on a monitoring utility would be its own bug.
- **Save logs** copies the current set to a folder the user picks.
- Deleting the log directory is safe at any time.

## 5. One log, not two

The front end forwards its events to Rust through a Tauri command rather than
writing its own file. A session then reads as a single ordered story, instead of
two files a reader has to interleave by timestamp — which is exactly the work
someone is trying to avoid when they open a log.

## 6. Making "no user data" a property rather than a promise

A rule enforced only by review erodes on the first busy afternoon. Three
mechanisms, in increasing strength:

**All logging goes through one module.** `src-tauri/src/log.rs` exposes
functions taking only counts, durations, enum kinds and booleans. There is no
`String`, `Path` or `PathBuf` parameter anywhere in its surface, so a path
cannot be logged because there is nowhere to put one.

**Errors log their variant, not their `Display`.** This is the rule most likely
to be broken by accident. `ChatError::SpawnFailed(String)` carries
`llama-server`'s stderr, which contains paths; `AcquireError` variants carry
file names. Logging `error` directly would leak both. Every error type gets a
`kind()` returning a `&'static str` — `spawn_failed`, `hash_mismatch`,
`not_a_gguf` — and only that is logged.

**A test scans emitted output.** The suite captures every line the logging paths
produce and fails on a path separator, an `@`, or a long digit run. It cannot
catch everything, but it catches the accidental `{:?}` of a struct that grew a
path field later — which is how this rule would realistically be broken.

## 7. Testing

- Each level emits what it should and nothing from a level above it.
- `warn` and `error` survive the lowest setting.
- The retention cap deletes oldest first and never the file being written.
- Every error type's `kind()` is exhaustive — a new variant fails to compile
  rather than logging as `unknown`.
- The scan test above, proven by deliberate breakage: log a path on purpose,
  confirm the test fails, restore.
- Front-end forwarding preserves ordering.

## 8. Consequences

- **New dependencies:** `tracing`, `tracing-subscriber`, `tracing-appender`.
  Licence and advisory checks run in CI already.
- **SECURITY.md gains a note** that logs exist, where they are, that they
  contain no user data by construction, and that they are therefore safe to
  attach to a bug report — the practical point of the whole decision.
- **Every error type grows `kind()`.** Mechanical, and it makes error handling
  more explicit at the call sites that currently format errors into strings.
- **The four stray `println!`s are removed.**
- **No ADR.** This makes no architectural choice that a later reader would need
  the reasoning for beyond what is written here.

## 9. Known risks

- **The scan test gives false confidence.** It catches path separators and digit
  runs, not a process name like `chrome`. The module boundary is the real
  control; the test is a net under it, and it should not be described as more.
- **Verbose logging on a monitoring app is high-volume.** The sampler ticks
  continuously, and `trace` on every tick could write steadily all day. The
  retention cap bounds the disk cost, but Verbose should be documented as
  something to turn on while reproducing rather than to leave on.
- **`kind()` on error enums is a wide, dull change** touching every error type
  in the workspace. It is the kind of change that is easy to get subtly wrong by
  copying the wrong string into a variant, and the exhaustiveness test exists
  for exactly that.
