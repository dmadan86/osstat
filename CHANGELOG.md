# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Cargo workspace with `osstat-core` (portable domain types) and
  `osstat-platform` (per-OS implementations selected at compile time).
- Tauri 2 desktop shell with a React, TypeScript, Vite and Tailwind front-end.
- Typed IPC boundary: TypeScript types are generated from the Rust structs by
  `ts-rs` and verified for staleness in CI.
- GitHub Actions CI across Windows, Linux and macOS: formatting, clippy,
  ESLint, type checking, unit tests and build verification.
- Release workflow producing `.msi`, `.dmg` (Intel and Apple Silicon),
  `.AppImage`, `.deb` and `.rpm` with SHA256 sums.
- Project governance: dual MIT/Apache-2.0 licensing, Contributor Covenant 2.1,
  security policy, DCO sign-off enforcement, and Conventional Commits.
- Architecture Decision Records for the eleven decisions the project is built on.

### Notes

- Nothing user-facing is implemented yet. The application opens a window showing
  its own build information and the capabilities that are still to come. See
  [ROADMAP.md](ROADMAP.md).
- The dual-licensing adds an Apache-2.0 option alongside the original MIT-only
  license, matching Rust ecosystem convention (ADR-010).

[Unreleased]: https://github.com/dmadan86/osstat/compare/main...HEAD
