# Roadmap

Milestones are executed in order. A milestone is not started until the previous
one's gate passes: code, tests, docs and green CI on Windows, Linux and macOS.

Dates are deliberately absent. The ordering and the gates are the commitment.

---

## M0 — Project foundation ✅

Cargo workspace, Tauri 2 shell, React/TypeScript front-end, CI matrix, and the
open-source governance the rest of the project depends on.

**Gate:** `just ci` green on all three platforms; the app window opens.

- [x] Workspace laid out per [ADR-003](docs/adr/ADR-003-platform-abstraction.md)
- [x] Typed IPC boundary with generated TypeScript ([ADR-002](docs/adr/ADR-002-language-split.md))
- [x] CI: formatting, clippy, ESLint, type checking, tests, build verification
- [x] Release workflow producing installers for all three platforms
- [x] Licensing, code of conduct, security policy, DCO, Conventional Commits
- [x] Architecture Decision Records

---

## M1 — System info and process management

The first real capabilities.

**Gate:** a process tree of 500 processes refreshes in under 50 ms, proven by a
committed Criterion benchmark.

- [ ] `SystemInfoProvider` and `ProcessProvider` traits with `sysinfo`-backed implementations
- [ ] Tauri commands plus a diffed event stream on a 2 s tick
- [ ] System info page: OS, kernel, uptime, CPU, memory, disks, GPUs, network
- [ ] "Copy as Markdown/JSON" for bug reports
- [ ] Process tree: PPID hierarchy, cumulative CPU/RAM/IO roll-ups, virtualized for 1000+ processes
- [ ] Search and filter by name, PID and user
- [ ] Kill flow: SIGTERM, then escalation to SIGKILL after a 5 s timeout with a second confirmation
- [ ] Per-OS critical-process list requiring a distinct extra confirmation
- [ ] Integration test: spawn a child process, find it in the tree, kill it, assert exit

---

## M2 — Port inspector

**Gate:** the end-to-end smoke suite covers the ports page.

- [ ] `SocketProvider` trait with a `netstat2` implementation, joined to the process table
- [ ] Table of protocol, local address, port, state, PID, process name and path
- [ ] Filter by port and process; listening sockets highlighted
- [ ] "Kill owning process" reusing the M1 kill flow, including the elevation path
- [ ] Integration test: bind a listener in-test, assert it appears with the right PID, kill it, assert the port frees

---

## M3 — Cleaning engine

The highest-risk milestone. Budget accordingly — this is where a bug costs
someone their files.

**Gate:** a 100k-file fixture tree scans in under 3 s, and property tests prove no
rule can escape its root.

- [ ] Manifest JSON Schema, TOML loader and validator with denylist and path-root enforcement
- [ ] Scanner: glob and environment-variable expansion, age and size filters, `guard_process` checks, parallel walk
- [ ] Deleter: trash-first, journalled, symlink-canonicalizing ([ADR-005](docs/adr/ADR-005-deletion-safety.md))
- [ ] 15–20 starter rule manifests across the three platforms
- [ ] Scan → preview → confirm → results UI
- [ ] Fixture tests per rule; `proptest` fuzzing for symlink escape and traversal
- [ ] Elevation helper for Windows and Linux ([ADR-006](docs/adr/ADR-006-privilege-elevation.md)); macOS may use `osascript` as an interim

---

## M4 — LLM runnability advisor

**Gate:** the runnability calculator has 100% branch coverage, and hardware
probing degrades gracefully with no GPU present.

- [x] `osstat-llm`: hardware probe (NVML, then `wgpu`, then fallback)
- [x] Model registry JSON with a schema, seeded with ~15 popular models × 4 quantizations
- [x] Runnability calculator as pure, exhaustively-tested functions
      (`calculator.rs` measures 100% region, line and function coverage)
- [x] Hardware card, fit matrix, and an explanation drawer showing the arithmetic
- [x] Apple Silicon reports unified memory correctly

### M4.1 — Inference runtime acquisition

The advisor says whether a model fits; this is the first step towards acting on
it. Split from the rest of local inference because it carries the security
decisions and has no visible payoff of its own — see
[ADR-012](docs/adr/ADR-012-local-inference-runtime.md) and
[the design](docs/superpowers/specs/2026-07-30-local-inference-runtime-design.md).

**Gate:** backend selection resolves on all three platforms and both
architectures, proven by test; a checksum mismatch is proven to leave nothing
executable behind.

- [x] `osstat-inference`: backend selection as pure functions over the ADR-008
      probe, exhaustively table-tested
- [x] `runtimes.json` pinning eleven upstream artifacts by SHA256, schema-validated,
      with drift tests in both directions against selection
- [x] Verified download: streamed, hashed as it arrives, moved into place only
      on a match, nothing left behind on failure
- [x] Extraction refusing any entry that would escape the runtime directory
- [x] Install store: two pinned versions coexist, sizes listed, deletable
- [x] Settings section offering both backends with their download sizes
- [ ] Manual acquisition verified by hand on Windows, Linux and macOS

### M4.2–M4.4 — Running a model

- [ ] **M4.2** Model acquisition: Hugging Face search, resumable GGUF download,
      local file import, GGUF header parsing reconciled against the registry
- [ ] **M4.3** Inference session: spawn with the advisor's `gpu_layers`,
      supervise, stream, cancel, tear down without orphaning the process
- [ ] **M4.4** Chat: conversation store, streaming UI, markdown, system prompt
      and sampling controls, live tokens/sec and context meters

---

## M5 — Hardening and release

**Gate:** v0.1.0 tagged with signed artifacts for all three platforms.

- [ ] End-to-end suite via `tauri-driver` across the matrix
- [x] Cold-start timing check with `hyperfine` against the under-2 s target
      (`just cold-start`; the script states what the figure excludes)
- [ ] Code signing: Windows Authenticode and macOS notarization
- [ ] Rule-authoring guide in `docs/`
- [ ] OpenSSF Best Practices badge application; `cargo deny` licence check
- [ ] Cut v0.1.0

---

## After v1

**Fast follows:** startup and autostart manager; disk space analyzer with a
treemap; dev-cache cleaner covering `node_modules`, cargo, pip, npm, gradle and
docker; duplicate file finder; opt-in scheduled cleaning using the OS scheduler
rather than a resident daemon; uninstaller with leftover detection.

**Designed for, not yet built:** a headless `osstat-cli`; a community rule
marketplace with signed rule packs; a sensors dashboard; Ollama integration for
one-click model pulls; localization (strings are externalized from the start).

**Backlog candidates:** a dev-machine mode aggregating every developer cache in
one screen; Docker cleanup; large and old file finder; browser privacy cleaner
with a cookie keep-list; service manager; per-process bandwidth; update checking
via winget, brew and apt; a composite system health score; a quick benchmark to
refine the LLM advisor's tokens-per-second estimates.

## Product goals these milestones serve

- Installer under ~20 MB, cold start under 2 s, idle RAM under 150 MB
- Native feel and native performance on all three platforms from one codebase
- Safe by default: nothing deleted without preview and confirmation, and
  recoverable when it is
- Open source to a professional standard

## Explicit non-goals for v1

No registry cleaning, no antivirus or malware scanning, no driver updating, no
telemetry of any kind, and no mobile targets. The first three are what gave this
category its reputation; the fourth is a trust commitment.
