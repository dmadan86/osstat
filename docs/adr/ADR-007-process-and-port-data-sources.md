# ADR-007: Process and port data sources

**Status:** Accepted

## Context

Process enumeration and socket tables are available through very different APIs
on each platform. Writing them from scratch means three implementations of
something well-solved; picking the wrong abstraction means fighting it later.

## Decision

- **Process enumeration, tree and statistics:** the `sysinfo` crate, refreshed on
  a 1–2 second tick, diffed, and pushed to the UI via Tauri events rather than
  polled from the front-end.
- **The tree** is built from parent PIDs and rendered as a collapsible hierarchy
  with cumulative CPU and memory roll-ups.
- **Ports and sockets:** the `netstat2` crate (or `listeners`) produces a socket
  table with owning PIDs, joined to the process table.
- **Killing:** `sysinfo`'s kill, sending SIGTERM first and escalating to
  SIGKILL/`TerminateProcess` after a timeout, with confirmation at each step.
  Permission failures route to the elevation flow in
  [ADR-006](ADR-006-privilege-elevation.md).
- **Critical processes** — `wininit.exe`, `launchd`, PID 1 and the rest of a
  per-OS list — require a distinct extra confirmation beyond the normal one.

## Rationale

Pushing diffs from Rust on a tick, rather than having the UI poll, keeps the
webview idle when nothing changes. That matters for the idle-RAM and
responsiveness goals with a thousand-plus process tree.

SIGTERM before SIGKILL gives processes a chance to flush state. Escalating
automatically without a second confirmation would silently turn a polite request
into a forced kill.

## Consequences

- The refresh tick and the diffing strategy are performance-critical. The M1 gate
  requires a 500-process tree refresh under 50 ms, enforced by a committed
  Criterion benchmark — a regression here shows up as a janky UI, which is hard
  to notice in review but obvious to a user.
- `sysinfo` becomes a load-bearing dependency for the two most visible features.
  If it proves insufficient on a platform, the trait boundary in
  [ADR-003](ADR-003-platform-abstraction.md) is what makes replacing it local.
- The critical-process list is per-OS data that must be maintained. An incomplete
  list means a user can kill something they should not have been able to kill
  with a single confirmation.
