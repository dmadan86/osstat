# ADR-002: Language split — Rust core, TypeScript and React front-end

**Status:** Accepted

## Context

Given Tauri (ADR-001), the split between backend and front-end responsibilities
needs to be explicit, and the boundary between them needs to be type-safe in both
directions.

## Decision

All system logic lives in Rust, as a workspace of crates. The UI is React 18 with
TypeScript, Vite and Tailwind. They communicate through Tauri commands (typed
IPC) and events (for live streams such as process and CPU updates).

IPC contracts are defined as Rust structs with `serde`, and their TypeScript
mirrors are **generated** rather than hand-written.

## Rationale

A clean separation lets the core crates back a standalone CLI (`osstat-cli`)
later without moving logic. That is a P2 goal, but it also makes headless
integration testing trivial today — the parts worth testing hardest are the parts
with no UI dependency.

Generating the TypeScript types means a Rust struct change that the front-end has
not followed is a compile error, not a runtime `undefined`.

## Consequences

- Every type crossing the IPC boundary must derive `serde` traits and be
  representable in TypeScript.
- Contributors changing an IPC type must regenerate bindings (`just bindings`)
  and commit the result. CI regenerates and fails on a stale diff, so this cannot
  be forgotten silently.
- The front-end holds no system logic at all. If a change wants to put logic
  there, it belongs in `osstat-core` instead.

## Implementation notes: ts-rs over specta

The original plan named "`ts-rs` or `specta`". We use **`ts-rs`**.

`tauri-specta` generates command signatures as well as types, which is strictly
more capable. But its Tauri 2-compatible line is still a release candidate, and
its stable release targets Tauri 1.x — pulling three pre-release crates into the
project's foundation trades a real stability risk for a modest ergonomic gain.

`ts-rs` is stable and generates types from a `cargo test` run, which gives us the
staleness check for free. It does not generate command signatures, so command
names are wrapped by hand in `ui/src/lib/ipc.ts` — one small, auditable file
where every command string lives.

Revisit this when `tauri-specta` for Tauri 2 reaches a stable release.
