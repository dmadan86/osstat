# ADR-009: Testing and CI strategy

**Status:** Accepted

## Context

A tool that deletes files and kills processes needs its correctness demonstrated,
not asserted. It also needs to prove itself on three operating systems, on a
budget of zero.

## Decision

| Layer       | Tooling                                                   | What it proves                              | Runs                  |
| ----------- | --------------------------------------------------------- | ------------------------------------------- | --------------------- |
| Unit        | `cargo test`, Vitest                                      | Engine logic, runnability maths, reducers   | Every pull request    |
| Property    | `proptest`                                                | No rule escapes its root; parser robustness | Every pull request    |
| Integration | `cargo test` with fixtures, spawned processes and sockets | OS behaviour end to end without the UI      | Every PR, 3-OS matrix |
| End-to-end  | `tauri-driver` with WebdriverIO                           | Real application flows including scan→clean | Nightly and release   |
| Performance | Criterion with a CI regression gate, `hyperfine`          | Scan speed, refresh latency, cold start     | Every PR, and release |
| Security    | `cargo audit`, `npm audit`, CodeQL                        | Dependency vulnerabilities                  | Every PR, weekly cron |
| Licence     | `cargo deny`                                              | Dependency licence compatibility (ADR-010)  | Every PR, from M5     |

Specifics:

- The cleaning engine is tested against **fixture directory trees**: create a
  temporary tree, run the rule, assert the exact set of surviving files. Not
  "roughly the right number deleted".
- Performance targets with committed benchmarks: a 100k-file fixture scan under
  3 s, a 500-process tree refresh under 50 ms, a port-table join under 20 ms.
- **CI uses only free GitHub-hosted runners** (`ubuntu-latest`, `windows-latest`,
  `macos-latest`), which are unlimited for public repositories.
- Release builds installers for all three platforms via `tauri-action` on tag
  push. `macos-latest` is Apple Silicon, so `macos-13` is added to the release
  matrix for Intel Macs.

## Rationale

The layering is chosen so the cheapest tests catch the most dangerous bugs. Rule
escape is a property test, not an example test, because examples only prove the
cases someone thought of — and the dangerous case is the one nobody thought of.

Fixture trees with exact surviving-set assertions catch the failure mode that
matters (deleted too much) rather than the one that is merely annoying (deleted
too little).

## Consequences

- Integration tests must run on the CI matrix, not just Linux, because the
  behaviours being tested are the platform-specific ones.
- Benchmarks in CI are noisy on shared runners. The regression gate needs a
  tolerance wide enough to avoid false failures and narrow enough to catch real
  regressions; expect to tune it.
- End-to-end tests are slow and flaky by nature, so they run nightly and at
  release rather than blocking every pull request.
