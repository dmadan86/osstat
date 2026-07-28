# ADR-003: Platform abstraction — a trait per capability, one implementation per OS

**Status:** Accepted

## Context

Every capability in osstat — cleaning, processes, ports, hardware — has
OS-specific implementations. Without a deliberate structure, `cfg(target_os)`
spreads through the codebase and the portable logic becomes untestable on any
single machine.

## Decision

Each capability is a trait declared in `osstat-core`. The `osstat-platform` crate
implements those traits in per-OS modules, selected at compile time by
`cfg(target_os)`.

```
crates/
  osstat-core/        traits, domain types, the cleaning-rule engine
  osstat-platform/    per-OS implementations (windows.rs / linux.rs / macos.rs)
  osstat-llm/         hardware probe, model registry, runnability maths   (M4)
  osstat-cli/         headless CLI over the same core                     (P2)
src-tauri/            the Tauri shell: commands, events, capabilities
ui/                   the React front-end
```

## Rationale

The portable core contains the logic most worth testing exhaustively — the
cleaning-rule engine and the runnability arithmetic — and it compiles and tests
on any CI runner with mock trait implementations. Platform crates are then tested
on the CI matrix, where the real OS is available.

Call sites depend only on the traits, so adding a platform means adding a module
and one `cfg` arm rather than touching every caller.

## Consequences

- Only one per-OS module is type-checked on a given host. `cargo fmt` sees all
  three, but `clippy` does not — which is precisely why CI runs the full
  Windows/Linux/macOS matrix rather than a single runner. A change that compiles
  locally can still break another platform, and only CI will say so.
- An unsupported target fails at compile time with an explicit message rather
  than a confusing cascade of missing symbols.
- `osstat-core` must stay free of platform code. The moment it needs a `cfg`, the
  logic belongs in `osstat-platform` behind a trait.
