# Governance

## Current model: single maintainer

osstat is maintained by [@dmadan86](https://github.com/dmadan86), who has final
say on scope, design and releases. This is written down not because the structure
is interesting at this size, but so contributors know what to expect before they
invest time.

This document will be replaced when the project outgrows it — see
[Growing beyond this](#growing-beyond-this).

## How decisions are made

**Ordinary changes** — bug fixes, new cleaning rules, UI work, tests — are decided
in the pull request. One maintainer approval merges.

**Architectural changes** get an Architecture Decision Record in `docs/adr/`
before the code. If you are about to change how deletion safety works, how
elevation is requested, or what the platform abstraction looks like, open an
issue proposing the ADR first. Writing the ADR is part of the change, not
paperwork after it.

**Scope decisions** — whether osstat should do a thing at all — are the
maintainer's call, guided by the non-goals in the ADRs. Some capabilities are
deliberately excluded (registry cleaning, driver updating, malware scanning), and
proposals to add them will be declined regardless of implementation quality.

## Areas that get stricter review

`CODEOWNERS` routes two areas to the maintainer specifically:

- **`rules/`** — cleaning manifests. A wrong glob deletes real user data, so
  these are reviewed against what the target application actually stores, not
  just for whether the TOML parses.
- **The elevation helper** — its command protocol is a security boundary
  (ADR-006, SECURITY.md). Widening it needs an explicit, documented decision.

## Becoming a maintainer

There is no formal process yet, and pretending otherwise would be dishonest at
this size. In practice: sustained, high-quality contributions and good judgement
in review discussions lead to an invitation. Reviewing other people's pull
requests well counts for at least as much as writing code.

## Growing beyond this

If the project reaches a point where one person is a bottleneck — roughly, more
than a handful of active contributors or a release cadence that stalls on review
— this document should be replaced with a multi-maintainer model: a small
maintainer group, an explicit tie-break rule, and a documented release-manager
rotation. Whoever notices the bottleneck first should open that issue.

## Code of conduct

Everyone participating is covered by the [Code of Conduct](CODE_OF_CONDUCT.md).
Enforcement is currently the maintainer's responsibility; reports go through the
private channels listed there.
