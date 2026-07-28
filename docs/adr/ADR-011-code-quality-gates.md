# ADR-011: Code quality gates and review process

**Status:** Accepted

## Context

`main` should always be releasable, and reviewers' attention should go to logic
and safety rather than formatting. That requires deciding, once, what is
mechanical and what is human.

## Decision

Every change reaches `main` only through a pull request that passes, in order:

1. **Local pre-commit hooks** (husky and lint-staged)
   - `rustfmt` on staged Rust; `clippy` across the workspace when Rust changed
   - Prettier on staged TypeScript, CSS, Markdown, YAML and JSON
   - Conventional Commit message linting via commitlint
2. **CI** (`ci.yml`): lint (rustfmt, clippy `-D warnings`, `cargo audit`,
   Prettier, ESLint, `tsc`) → unit tests → functional tests → build verification,
   matrixed across the three free GitHub-hosted runners
3. **Human review**: at least one approval, enforced by branch protection.
   `CODEOWNERS` routes `rules/**` (cleaning manifests) and the elevation helper
   to the maintainer.
4. **Merge**: squash only, with a Conventional Commit pull request title
   (title-lint enforced) — keeping `main` linear and changelog-generatable.

### Formatting ownership

No overlap, no debates:

- Rust → `rustfmt`, default style, checked in CI
- TypeScript, CSS, JSON, YAML, Markdown → **Prettier**, config in the repository root
- Generated files (`ui/src/bindings/`) are formatted by their generator and
  excluded from Prettier

Reviewers do not comment on formatting.

### Lint policy

Clippy runs with `pedantic` enabled and `-D warnings`, so a warning fails the
build. `unwrap`, `expect`, `panic!`, `todo!` and `unimplemented!` are lint errors
outside tests; test modules opt out locally with an `allow` attribute rather than
the policy being relaxed globally.

### Branch protection on `main`

Configured in repository settings and documented here:

- Require a pull request; require the status checks `Lint`, `Test`, `Build` and
  `DCO` to pass
- Require at least one approving review; dismiss stale approvals on new commits
- Require linear history; forbid force-push
- Require DCO sign-off

## Consequences

- Pull request latency rises by a few minutes. In exchange, `main` is always
  releasable and review comments are about substance.
- Contributors occasionally hit a clippy pedantic lint that is wrong for their
  case. The answer is a local `#[allow]` with a comment explaining why, not
  disabling the lint globally.

## Superseded decision: automated PR review

The original plan included a `code-review.yml` workflow running an LLM-based
reviewer over every non-draft pull request via AWS Bedrock.

This was **dropped before implementation**. It required AWS credentials in
repository secrets — an operational and cost commitment before the project has
any code to review — and its role (catching missing tests and unsafe deletion
paths) is already covered by the mandatory clippy lints, the test-coverage
requirements in `CONTRIBUTING.md`, and `CODEOWNERS` routing for the two most
dangerous areas.

Revisit if pull request volume outgrows the maintainer's review capacity. Should
it return, it belongs as an advisory check that never blocks a merge.
