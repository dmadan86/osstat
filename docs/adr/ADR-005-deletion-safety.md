# ADR-005: Deletion safety model

**Status:** Accepted

## Context

The worst-case bug in this project is not a crash or a wrong number on a screen.
It is deleting a user's files. Every design choice around deletion should be read
in that light.

## Decision

1. **Scan and delete are separate phases.** A scan produces a manifest of
   candidate files with sizes. The UI shows a preview. Nothing is removed until
   the user confirms.
2. **Move to trash by default** — the Recycle Bin, Trash, or GIO trash via the
   `trash` crate. Permanent deletion is opt-in, per run.
3. **A hard denylist that no manifest can override**: system roots (`C:\Windows`,
   `/System`, `/usr`, `/etc`, `/boot`), user document folders, anything outside
   the expanded roots of the rule's own patterns, and any symlink target that
   escapes its pattern root. Paths are resolved with `canonicalize` _before_
   deletion, not after matching.
4. **Every delete run writes a journal** — a JSON log of what was removed, when,
   and by which rule — to the application data directory.
5. **Dry-run mode** is available in the CLI and exercised in every integration
   test.

## Rationale

Each item addresses a distinct failure mode: a wrong rule (preview), a correct
rule the user did not want (trash), a hostile or buggy pattern (denylist and
canonicalization), and "what happened to my file?" after the fact (journal).

Canonicalizing before deleting rather than trusting the matched path is the
specific defence against a symlink planted inside a cache directory pointing
somewhere important.

## Consequences

- Deletes are slightly slower: canonicalization is a syscall per path, and
  journaling is an extra write. This is the correct trade for a tool whose
  worst-case bug is data loss.
- Trash is not free — moving to the Recycle Bin consumes space until it is
  emptied, and users cleaning a full disk may need the permanent option. It stays
  opt-in and per-run rather than a sticky setting.
- Property-based tests (`proptest`) must prove the root-escape invariant rather
  than merely sampling it. "No rule may ever match outside its root" is the
  single most important property in the codebase.
