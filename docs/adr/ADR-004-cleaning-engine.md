# ADR-004: Cleaning engine — declarative rule manifests, not hardcoded paths

**Status:** Accepted

## Context

Cleaning in the CCleaner sense is fundamentally "hundreds of application-specific
path patterns". Encoding those in Rust makes community contribution painful:
adding support for one more editor's cache would require a Rust developer, a
build, and a review of imperative code.

## Decision

Cleaning rules are **TOML manifests**, one per application per OS — similar in
spirit to BleachBit's CleanerML but simpler. The Rust engine interprets them.

A manifest supports glob patterns, environment-variable expansion (`%APPDATA%`,
`$XDG_CACHE_HOME`, `~/Library/Caches`), age filters, size filters, and a
"this process must not be running" guard.

```toml
[cleaner]
id = "vscode-cache"
app = "Visual Studio Code"
category = "dev"

[[rule]]
os = ["windows"]
path = "%APPDATA%/Code/Cache/**"
action = "delete"
guard_process = "Code.exe"

[[rule]]
os = ["linux", "macos"]
path = "~/.config/Code/Cache/**"
action = "delete"
guard_process = "code"
```

## Rationale

A contributor adds a cleaner with a pull request containing one TOML file and a
test fixture — no Rust knowledge required. Rules are auditable, diffable, and can
be schema-validated in CI. A reviewer can see exactly what a rule will delete by
reading it, which is not true of imperative code.

## Consequences

- **The engine must be hardened against manifest abuse.** Rules are data, and
  data can be wrong or hostile: path traversal, symlink escape, and absolute
  paths outside the allowed roots are all attacks a manifest can attempt. The
  denylist and root enforcement in [ADR-005](ADR-005-deletion-safety.md) are not
  optional extras — they are what makes this decision safe.
- A JSON Schema for the manifest format ships with the project and is validated
  in CI, so a malformed rule fails before review rather than at runtime.
- The `os` values in a manifest (`"windows"`, `"linux"`, `"macos"`) are part of
  the public format and must stay in sync with `PlatformId::as_str`.
- Manifest schema changes require a documentation update in the same pull
  request, because external contributors write against the documented format.
