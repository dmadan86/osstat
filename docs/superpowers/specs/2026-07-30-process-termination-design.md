# Ending a process, from the tree and from the port table

**Date:** 2026-07-30
**Status:** Implemented.
**Supersedes:** the deliberate absence recorded in `Processes.tsx`, `Ports.tsx`
and `App.test.tsx`. **Depends on:** ADR-003, ADR-006, ADR-007.
**Delivers:** the four unticked kill items in ROADMAP M1 and M2.

**Amendment during implementation:** the critical-process list is
`critical-processes.json`, not `.toml` as §3 and §5 originally specified.
ADR-004's "rules are data" precedent holds; the format does not. ADR-004 is
about cleaning-rule manifests, which users author; this list is internal data,
and the two internal data files already in the tree — `models.json` and
`runtimes.json` — are JSON with a sibling JSON Schema and a test validating
one against the other, which is the precedent this list actually follows.
`serde_json` was already a workspace dependency; `toml` would have been new.

## Context

The process tree and the port table both show you a process you might want gone,
and neither offers a way to end it. That was a decision, not an omission, and it
is recorded three times:

- `Processes.tsx`: "deliberately no kill affordance at all — not even a disabled
  one, which would promise something this phase does not deliver and invite a
  bug report instead of setting an expectation."
- `Ports.tsx`: defers to M1, because "adding it now would mean designing the
  destructive-action decisions that M1 owns, not this page."
- `App.test.tsx`: a test named "offers no way to kill a process in this phase"
  that actively asserts the absence.

This spec is where those decisions get made, so the absence can end.

The concrete use is narrower than "process management" suggests: a dev server is
holding port 3000, or a build has hung. Both are processes the user already owns.

## Goals

1. End a process you own, from the process tree or the port table, on all three
   platforms.
2. Give it a chance to exit cleanly first, and mean it on every platform.
3. Never end a process the user did not select.
4. Make a permission failure legible rather than a dead end.

## Non-goals

- **Elevation.** ADR-006's helper does not exist. Nothing here elevates, prompts
  for credentials, or relaunches anything. A process osstat cannot end says so.
  See §6.
- **Killing a subtree.** One process per action. See §1.
- **A process's children being reparented, adopted or followed.** Orphaning is
  named in the confirmation, not managed.
- **Suspend, resume, renice, or priority changes.** Different feature, different
  risks.
- **Any port-specific mechanism.** M2's item is the M1 flow reached from another
  table, not a second implementation.

---

## 1. Decisions taken during design

Recorded because each closed off an option that will look attractive again later.

| Decision                                      | Rejected alternative            | Why                                                                                                                                                                                                                                                                       |
| --------------------------------------------- | ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Kill takes a `ProcessKey`                     | Kill takes a PID                | Five seconds separate the two steps, and a PID freed in that window can be reused. See §2 — this is the single most important property in the spec.                                                                                                                       |
| Own processes only, no elevation              | Build ADR-006's helper first    | The helper is a second shipped binary with a versioned protocol, a Polkit policy and macOS registration. It roughly triples the work, and killing your own process needs none of it.                                                                                      |
| `WM_CLOSE` then `TerminateProcess` on Windows | `sysinfo`'s `kill()` everywhere | The graceful step posts `WM_CLOSE`; the forceful step calls `sysinfo::Process::kill`, which shells out to `taskkill.exe /PID <pid> /F` rather than calling `TerminateProcess` directly. Using it for both steps would make them identical while the UI claimed otherwise. |
| One process, not the subtree                  | "End this and its children"     | Multiplies the blast radius of a misclick. The roadmap's integration test is single-process.                                                                                                                                                                              |
| The 5 s wait lives in the front end           | A pending-kill timer in Rust    | Backend state would have to survive the app quitting mid-wait, handle two requests for one PID, and re-check identity anyway. Each command becomes one action for which consent was already given.                                                                        |
| Critical list is JSON data                    | TOML, or a hardcoded array      | ADR-004's precedent — rules are data, reviewed as data — holds; the format named in the original spec did not. See the amendment note above.                                                                                                                              |
| Match on name **and** path                    | Name alone                      | A user's own `explorer.exe` in Downloads is not the real one.                                                                                                                                                                                                             |

## 2. Identity: why this takes a `ProcessKey`

`osstat-core` already has it, and the diff stream already depends on it:

```rust
pub struct ProcessKey {
    pub pid: u32,
    pub started_at: u64,
}
```

`ProcessDiff::removed` carries it rather than a bare PID, with the reason stated
in the source: "when a PID is recycled within one tick, the same number appears
here as removed and in `added` as a new process, and the start time is what tells
them apart."

The kill flow has a far worse version of that problem. Between the graceful
attempt and the forceful one there are five seconds, which is ample for a PID to
be freed and reused. Sending `TerminateProcess` to a recycled PID would end **a
process the user never selected**, with no error and no way to tell afterwards.

So:

- Every termination call takes a `ProcessKey`.
- The platform layer re-reads that PID's start time **immediately before
  signalling** and refuses if it does not match.
- The refusal is its own error variant, not a generic failure. If it ever fires
  in the field it means something worth knowing happened.

This is cheap — one `stat` or `OpenProcess` — and it is the difference between a
destructive tool that is safe and one that is usually safe.

## 3. Where the code lives

Per ADR-003 and AGENTS.md's placement rules:

**`osstat-core`** — portable only:

- `ProcessController` trait: `terminate(&self, key: ProcessKey, mode: TerminationMode) -> Result<Termination>`
- `TerminationMode::{Graceful, Forceful}`
- `Termination::{Signalled, AlreadyGone, NoWindowToClose}` — `AlreadyGone` is a
  success the UI must not confuse with `Signalled`, because it means no waiting
  and no second step. `NoWindowToClose` is Windows telling the UI not to make
  the user wait 5 s for nothing.
- Critical-process matching: pure functions over the deserialized list.

**`osstat-platform`** — the OS-specific half:

- Unix: `sysinfo`'s `kill_with` with `SIGTERM` or `SIGKILL`, not `libc::kill`
  directly, so the crate keeps its `#![deny(unsafe_code)]`.
- Windows: `EnumWindows` plus `GetWindowThreadProcessId` to find top-level windows
  belonging to the PID, `PostMessageW(WM_CLOSE)` to each; `sysinfo`'s `kill()`
  (`taskkill.exe /PID <pid> /F`) for the forceful mode. Needs the `windows`
  crate, which is new to this project and must clear `cargo deny`.

**`src-tauri`** — one adapter, `terminate_process`, no logic.

**`ui`** — one confirmation component used by both pages. Neither page implements
a dialog of its own; M2's item is M1's flow reached from a second table.

**`crates/osstat-core/critical-processes.json`** — the list, with a schema and a
test validating it, following `models.json` and `runtimes.json`. See the
amendment note at the top of this document — the original spec named
`.toml`.

In `osstat-core` rather than `osstat-platform` despite being per-OS content: it
is _data about_ operating systems, not code that runs differently on them.
Loading and matching are portable, so a Windows entry can be tested from Linux —
and must be, because otherwise two thirds of the list is only ever exercised on
one runner.

## 4. The flow

```
User clicks "End process"
  → confirm: name, PID, "its children will keep running, reparented"
  → critical? a second, differently worded confirmation naming what breaks
  → terminate(key, Graceful)
      → Unix: SIGTERM
      → Windows: WM_CLOSE to each top-level window
      → Windows, no windows found: return NoWindowToClose
  → NoWindowToClose: offer the forceful step now, saying why
  → otherwise: watch the process stream for up to 5 s
      → key disappears: done. No second dialog, no notification.
      → still present: "It did not exit. End it forcefully?"
          → terminate(key, Forceful)   [SIGKILL / TerminateProcess]
```

The graceful step succeeding quietly is the common case, and it gets no dialog at
all. A tool that congratulates you for each routine action is a tool people learn
to click through, which is exactly the habit a destructive feature must not build.

The wait watches the existing `processes:tick` stream rather than polling a new
command: the tree is already being diffed on a 2 s tick, and `removed` already
carries the key being waited on.

Two consequences of reusing that stream, both worth stating rather than
discovering:

**The 5 s is really "the first tick at or after 5 s".** The tick is 2 s, so
detection lands somewhere in 5–7 s, and the sampler drops to a slower rate when
the window is not in the foreground. The confirmation must therefore be worded
as "it has not exited yet" rather than promising a precise interval — and the
wait must have an upper bound (15 s) after which it offers the forceful step
regardless, so a paused sampler cannot strand the user in a spinner.

**The Ports page does not follow the tick, but the subscription is app-wide.**
`Ports.tsx` fetches on demand by design (`src-tauri/src/ports.rs` explains why),
so it has no stream of its own to watch. It does not need one: `processes:tick`
is emitted to the window regardless of which page is showing, so the shared
confirmation component subscribes to it directly rather than depending on the
page that opened it. This is the concrete reason the dialog is one component
rather than two — a second copy on the Ports page would have had to invent a
polling loop for a signal that was already arriving.

## 5. Critical processes

`critical-processes.json`, one entry per platform per process, each with a name
and the paths it is expected at:

| Platform | Examples                                                                |
| -------- | ----------------------------------------------------------------------- |
| Windows  | `explorer.exe`, `csrss.exe`, `lsass.exe`, `wininit.exe`, `services.exe` |
| Linux    | `systemd`, `init`, `dbus-daemon`, `Xorg`, `gnome-shell`                 |
| macOS    | `launchd`, `WindowServer`, `loginwindow`, `Finder`                      |

Both must match. A binary named `explorer.exe` in `~/Downloads` is not the shell
and must not inherit its protection — which would be the wrong way round: the
unknown binary would become _harder_ to kill than the real system process.

**This guards a misclick, not an adversary.** Name matching is spoofable in
principle; an attacker who can already place a binary on the machine has won
long before this list matters. Saying so here prevents someone later mistaking it
for a security control and building on it.

Most of these also fail with `PermissionDenied` because they run as SYSTEM or
root. The list exists for the ones that do not: your own session's shell or
window manager, where a confirmed kill logs you out.

## 6. Failure modes

| Failure                              | Behaviour                                                                                                                                                                                                                         |
| ------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Not permitted                        | "This process belongs to another user. osstat cannot end it yet." Distinct error variant preserved through every layer, per ADR-006 — it is the signal that will unlock "Retry elevated". No prompt, no elevation, no pretending. |
| **Identity mismatch**                | **Refuse. Report that the process ended and its PID was reused. Never signal.**                                                                                                                                                   |
| Already gone                         | Success. The user wanted it gone; it is gone.                                                                                                                                                                                     |
| No window to close (Windows)         | Not a failure. `NoWindowToClose`, and the UI offers the forceful step immediately.                                                                                                                                                |
| Graceful sent, still alive after 5 s | Not a failure. The second confirmation.                                                                                                                                                                                           |
| Forceful failed                      | Report the OS error with the PID and name. There is no third step.                                                                                                                                                                |

## 7. Testing

**Pure:**

- Critical matching: right name and right path matches; right name and wrong path
  does not; wrong name at a protected path does not; per-platform lists are
  matched against the correct platform only.
- `critical-processes.json` validated against its schema.

**Integration** — the test ROADMAP M1 names, delivered in
`crates/osstat-platform/tests/terminate.rs`:

- Spawn a child process, find it in the tree, terminate it, assert it exited.
- Spawn, terminate gracefully, assert it exits without needing the forceful step.
- **Identity refusal:** build a key with a real PID and a wrong `started_at`,
  assert termination refuses and the process is still running afterwards.
- Ending something already gone is a success, not an error.

**M2's integration test — not delivered in this change.** Wiring the Ports page
into the confirmation flow (resolving a real `ProcessKey` from the tree by PID,
disabling the control when it cannot) is covered by front-end tests against a
mocked backend, but no test binds a real listener, ends its owning process, and
asserts the port frees. That gap is named rather than silently left; see §9.

**Front end:**

- Nothing is terminated without a confirmation.
- A critical process requires the second, distinct confirmation.
- The forceful prompt appears only after the graceful step has been given its 5 s.
- `NoWindowToClose` skips the wait.
- `App.test.tsx`'s "offers no way to kill a process in this phase" is **replaced**
  by a test pinning the confirmation. The absence was pinned deliberately; what
  supersedes it deserves the same treatment rather than a quiet deletion.

**Not tested automatically:** killing a process osstat does not own, and killing
anything on the critical list. Both are verified by hand and named as such.

## 8. Documents this obliges

- **ROADMAP** M1: three items ticked. M2: "Kill owning process" ticked; its
  own integration test (bind a listener, end it, assert the port frees) is
  not delivered in this change and stays unticked — see §7 and §9.
- **SECURITY.md** needs no new threat. osstat gains no privilege here — it ends
  processes the user could already end from Task Manager or `kill`. The privacy
  and elevation sections are unchanged, and that is worth stating explicitly in
  the pull request so a reviewer does not have to work it out.
- **ADR-006** unchanged and unwidened. When the helper lands, its first protocol
  verb is "terminate this `ProcessKey`" — the type this spec establishes, which
  is why it must carry the start time from the beginning rather than gaining it
  later.

## 9. What "done" means

- [x] `ProcessController` in `osstat-core` with per-OS implementations.
- [x] Termination refuses on identity mismatch, proven by test.
- [x] `WM_CLOSE` path on Windows; `SIGTERM`/`SIGKILL` on Unix.
- [x] `critical-processes.json`, schema-validated, matched on name and path.
- [x] Confirmation component used by Processes and Ports alike.
- [x] `PermissionDenied` reaches the UI intact and reads honestly.
- [x] ROADMAP's M1 integration test passes.
- [ ] ROADMAP's M2 integration test (bind a listener, end it, assert the port
      frees) — not delivered; see §7. Ports.tsx's own wiring is covered by
      front-end tests against a mocked backend instead.
- [x] `App.test.tsx`'s absence test replaced, not deleted.
- [x] `just ci` green — verified on Windows, where this change was built.
      Linux and macOS are exercised by CI on the pull request, not locally.
- [x] `cargo deny` green with the `windows` crate added.
