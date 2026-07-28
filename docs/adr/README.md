# Architecture Decision Records

These records capture the decisions osstat is built on: what was decided, what
else was considered, and what the decision costs. They are written so that
someone arriving in a year — including the person who wrote them — can tell
whether a decision still holds.

A record is **immutable once accepted**. When a decision changes, add a new
record that supersedes the old one and mark the old one `Superseded by ADR-NNN`.
Do not rewrite history in place; the reasoning that turned out to be wrong is
often the most useful part.

Write a new ADR before the code when a change affects how deletion safety works,
how privileges are requested, what the platform abstraction looks like, or any
other decision that would be expensive to reverse.

| ADR                                              | Decision                                             | Status   |
| ------------------------------------------------ | ---------------------------------------------------- | -------- |
| [001](ADR-001-application-framework.md)          | Tauri 2 as the application framework                 | Accepted |
| [002](ADR-002-language-split.md)                 | Rust core, TypeScript and React front-end            | Accepted |
| [003](ADR-003-platform-abstraction.md)           | A trait per capability, one implementation per OS    | Accepted |
| [004](ADR-004-cleaning-engine.md)                | Declarative TOML rule manifests, not hardcoded paths | Accepted |
| [005](ADR-005-deletion-safety.md)                | The deletion safety model                            | Accepted |
| [006](ADR-006-privilege-elevation.md)            | Per-operation elevation; the app never runs elevated | Accepted |
| [007](ADR-007-process-and-port-data-sources.md)  | Process and socket data sources                      | Accepted |
| [008](ADR-008-hardware-probe-and-llm-advisor.md) | Hardware probing and LLM runnability estimation      | Accepted |
| [009](ADR-009-testing-and-ci.md)                 | Testing and CI strategy                              | Accepted |
| [010](ADR-010-licensing-and-governance.md)       | Dual MIT/Apache-2.0 licensing and governance         | Accepted |
| [011](ADR-011-code-quality-gates.md)             | Code quality gates and the review process            | Accepted |
