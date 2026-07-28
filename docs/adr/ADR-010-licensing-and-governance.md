# ADR-010: Licensing and governance

**Status:** Accepted

## Context

The project needs a license that maximises adoption, fits the Rust ecosystem, and
does not create friction for contributors — plus a commit and release convention
that supports automated changelogs.

## Decision

- **Dual licensed MIT OR Apache-2.0**, at the user's option.
- **Conventional Commits**, with `release-plz` or changesets generating changelogs
  and SemVer version bumps automatically.
- **DCO sign-off** rather than a Contributor License Agreement.

## Rationale

Dual MIT/Apache-2.0 is Rust ecosystem convention. It is maximally permissive
while the Apache-2.0 option supplies an explicit patent grant, which some
corporate users require before they can adopt a dependency. Offering both means
neither preference blocks anyone.

The DCO is a one-line sign-off certifying you have the right to submit the work.
A CLA requires contributors to read a legal agreement and often to assign rights,
which measurably reduces drive-by contributions — exactly the contributions this
project wants for cleaning rules.

Conventional Commits are enforced mechanically rather than by review nagging, and
they make the changelog a build artifact rather than a chore.

## Consequences

- Every source contribution is dual-licensed. Contributors are told this in
  `CONTRIBUTING.md`, and it is restated in the README's license section.
- Dependencies must be licence-compatible. `cargo deny` checks this in CI; a
  copyleft dependency would be a licensing problem, not just a preference.
- Because the project squash-merges, the **pull request title** becomes the
  commit on `main` and must itself be a valid Conventional Commit. This is
  checked in CI, since a bad title silently corrupts the generated changelog.
- The repository originally carried an MIT-only license. Adding the Apache-2.0
  option was done while the maintainer was the sole copyright holder; doing so
  later would have required agreement from every contributor.
