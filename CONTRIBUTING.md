# Contributing to osstat

Thanks for considering a contribution. This document covers how to get set up,
what the review process looks like, and the few rules that are non-negotiable
because of what this tool does to people's filesystems.

## Ways to contribute that need no Rust

- **Cleaning rules.** A rule is one TOML file plus a test fixture. This is the
  highest-value contribution and the easiest place to start. See
  [Authoring cleaning rules](#authoring-cleaning-rules).
- **LLM registry entries.** Model sizes and quantizations live in a JSON file.
- **Bug reports** with the output of the app's "Copy system info" button.
- **Documentation**, including correcting anything in `docs/adr/` that has drifted
  from the code.

## Development setup

Prerequisites are listed in the [README](README.md#build-from-source). Then:

```sh
just setup           # npm ci
just dev             # run the app with hot reload
just test            # Rust + frontend tests
just lint            # formatters, clippy, eslint, tsc
just ci              # everything a pull request must pass
```

Run `just` with no arguments to list every target.

### Project layout

| Path                     | What lives there                                                |
| ------------------------ | --------------------------------------------------------------- |
| `crates/osstat-core`     | Portable domain types, traits and engines. No OS-specific code. |
| `crates/osstat-platform` | Per-OS implementations, selected by `cfg(target_os)`.           |
| `src-tauri`              | The desktop shell: IPC commands, events, capabilities.          |
| `ui`                     | React + TypeScript front-end.                                   |
| `ui/src/bindings`        | **Generated.** Never edit by hand — run `just bindings`.        |
| `docs/adr`               | Architecture Decision Records.                                  |

If a change makes you want to put system logic in `src-tauri`, it probably
belongs in `osstat-core` (portable) or `osstat-platform` (per-OS) instead. The
shell is deliberately a thin adapter so the same core can back a CLI later.

### Changing anything that crosses the IPC boundary

TypeScript types are generated from the Rust structs by `ts-rs` (ADR-002). After
changing a type that a command returns:

```sh
just bindings        # regenerates ui/src/bindings/
```

Commit the regenerated files. CI regenerates them too and fails if the committed
output is stale, so a forgotten `just bindings` is caught before review.

## The rules that are not negotiable

These exist because the worst-case bug in this project is "deleted the user's
files", not "rendered the wrong colour".

1. **Every new module ships with tests in the same pull request.** Not a
   follow-up issue.
2. **No `unwrap()`, `expect()` or `panic!()` outside tests.** These are lint
   errors in CI. Return an error and let the caller decide.
3. **Every filesystem-mutating code path needs a dry-run branch** and fixture
   tests covering it _before_ it is wired to the UI.
4. **Deletion respects the safety model in [ADR-005](docs/adr/ADR-005-deletion-safety.md).**
   Trash by default, canonicalize before deleting, honour the hard denylist,
   write a journal entry. A change that weakens any of these needs a
   corresponding ADR update, not just a code review.
5. **Never widen the elevation helper's command protocol** without updating
   [SECURITY.md](SECURITY.md) and [ADR-006](docs/adr/ADR-006-privilege-elevation.md).
   The helper accepts a narrow, versioned set of operations — that narrowness is
   the security property.
6. **Update the rule-authoring docs whenever the manifest schema changes.**

## Authoring cleaning rules

Cleaning rules are TOML manifests in `rules/`, interpreted by the engine rather
than compiled in (ADR-004). A rule looks like this:

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

To contribute one:

1. Add the TOML file under `rules/`.
2. Add a fixture directory under `crates/osstat-core/tests/fixtures/` showing the
   tree the rule should and should not match.
3. Run `just test`. The engine validates every manifest against the JSON schema
   and rejects paths outside the allowed roots.

Rules are reviewed by the maintainer specifically (see `CODEOWNERS`) because a
wrong glob deletes real data. Expect questions about what an app actually stores
in a path before a rule is merged.

> The full engine lands in milestone M3. Until then, rule contributions are
> welcome as drafts but cannot be tested end to end.

## Commits and pull requests

**Conventional Commits** are enforced by a hook and by CI:

```
feat(processes): add cumulative CPU roll-up to the tree view
fix(cleaner): resolve symlinks before deleting
rules(browsers): add Firefox profile cache for Linux
docs: clarify the elevation flow in ADR-006
```

Allowed types: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`, `ci`,
`chore`, `revert`, and `rules` (for cleaning manifests). Scopes are kebab-case.

Because the project squash-merges, **the pull request title becomes the commit on
`main`** and must itself be a valid Conventional Commit. CI checks this.

### Sign your commits (DCO)

Every commit needs a `Signed-off-by` line certifying you have the right to submit
the work under the project's license. Use `-s`:

```sh
git commit -s -m "feat(ports): join socket table to the process list"
```

There is no CLA. The [Developer Certificate of Origin](https://developercertificate.org/)
sign-off is all that is required, and CI enforces it.

### What happens to your pull request

1. Pre-commit hooks have already run formatters, clippy and commitlint locally.
2. CI runs lint, unit tests, functional tests and a build verification on
   Windows, Linux and macOS.
3. A maintainer reviews. Changes under `rules/` and to the elevation helper are
   routed to the maintainer automatically.
4. Squash merge. Release tooling handles the changelog and version bump.

Formatting is owned entirely by `rustfmt` (Rust) and Prettier (everything else).
Reviewers will not comment on formatting, and neither should you — run
`just fmt`.

## Reporting security issues

Do not open a public issue. Follow [SECURITY.md](SECURITY.md).

## License

By contributing, you agree that your contributions will be dual-licensed under
MIT and Apache-2.0, matching the project.
