# ADR-006: Privilege elevation — per-operation, never run elevated

**Status:** Accepted

## Context

Killing another user's process, cleaning system temp directories, and closing
privileged ports all need administrator or root rights. The easy approach —
relaunch the whole application elevated — means a webview-hosting process runs
with full system privileges for its entire lifetime.

## Decision

**The application always runs unprivileged.** An operation that fails with a
permission error surfaces an explicit "Retry elevated" affordance, which invokes
the OS-native elevation for that single operation:

| Platform | Mechanism                                                                                  |
| -------- | ------------------------------------------------------------------------------------------ |
| Windows  | The `runas` verb via `ShellExecuteExW` on a small helper binary (UAC prompt)               |
| Linux    | `pkexec` (Polkit) with a shipped policy file, falling back to a `sudo -A` prompt           |
| macOS    | A privileged helper registered with `SMAppService` (or an `osascript` admin prompt for v1) |

The helper accepts a **narrow, versioned command protocol** — "kill this PID",
"delete the files in this validated manifest ID" — and **never arbitrary paths or
commands**.

## Rationale

This is more engineering than relaunching as admin. It is also the difference
between a trustworthy system tool and a privilege-escalation vector: a
long-running elevated process that renders web content is a large attack surface,
and a helper that accepts arbitrary paths is a rootkit with extra steps.

The narrowness of the protocol _is_ the security control, not a detail of it.

## Consequences

- This is a **P0 security requirement**, not a nice-to-have. Shipping the kill
  and clean features without it would be shipping the vulnerability.
- `osstat_core::Error::PermissionDenied` must be preserved distinctly through
  every layer. Flattening it into a generic I/O error breaks the only signal the
  UI has for offering elevation, so the error type recognises
  `ErrorKind::PermissionDenied` arriving through the I/O variant as well.
- Widening the helper protocol requires updating [SECURITY.md](../../SECURITY.md)
  and this record. `CODEOWNERS` routes helper changes to the maintainer.
- Each elevated operation costs the user a prompt. That friction is intentional
  and should not be optimised away with caching or a "don't ask again" option.
