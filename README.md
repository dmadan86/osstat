# osstat

[![CI](https://github.com/dmadan86/osstat/actions/workflows/ci.yml/badge.svg)](https://github.com/dmadan86/osstat/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

A free, open-source system utility for Windows, Linux and macOS. It inspects your
machine, manages processes and ports, safely reclaims disk space, and tells you
which local LLMs your hardware can actually run.

> **Status: pre-alpha.** Milestone M0 (project foundation) is complete: the app
> builds and opens a window on all three platforms, but none of the capabilities
> below are implemented yet. Follow [ROADMAP.md](ROADMAP.md) for what is landing
> when. There is no release to install yet.

## What it does

| Capability          | What it gives you                                                              | Status |
| ------------------- | ------------------------------------------------------------------------------ | ------ |
| **System info**     | OS, kernel, CPU, memory, disks, GPUs, network — copyable as Markdown or JSON   | M1     |
| **Process manager** | Full process tree with per-process CPU/RAM/IO and permission-aware kill        | M1     |
| **Port inspector**  | Which process holds which port, and close it on demand                         | M2     |
| **System cleaner**  | Caches, logs, browser and dev-tool junk — previewed before anything is removed | M3     |
| **LLM advisor**     | Detects your GPU/VRAM/RAM and tells you which models and quantizations fit     | M4     |

## Why another cleaner

Most tools in this category are heavy, closed, and ask you to trust them with
delete permissions on your whole disk. osstat takes the opposite position:

- **Nothing is deleted without a preview.** Scanning and deleting are separate
  phases, and the default action is move-to-trash, not permanent removal.
- **It never runs elevated.** Operations that need admin rights ask for them one
  at a time, for that operation only. See [ADR-006](docs/adr/ADR-006-privilege-elevation.md).
- **No telemetry. None.** osstat makes no network requests except when you
  explicitly ask it to (for example, checking the model registry for updates).
  There is no analytics SDK, no crash reporter, and no phone-home.
- **The rules are readable.** What gets cleaned is defined in TOML manifests you
  can read, diff and contribute to — not compiled into a binary.
- **It is small.** A Rust backend with the OS's own webview, not a bundled browser.

Deliberately **not** included: registry "cleaning", driver updating, and malware
scanning. Those are the features that give this category its bad reputation, and
none of them measurably help.

## Install

No releases yet. Build from source (below); installers for `.msi`, `.dmg`,
`.AppImage`, `.deb` and `.rpm` arrive with v0.1.0.

## Build from source

Prerequisites:

- [Rust](https://rustup.rs) — the pinned toolchain in `rust-toolchain.toml` installs automatically
- [Node.js](https://nodejs.org) 20 or newer
- [just](https://github.com/casey/just) — `cargo install just`
- Platform build dependencies, per the [Tauri prerequisites](https://tauri.app/start/prerequisites/):
  - **Windows** — Visual Studio Build Tools (C++ workload) and the WebView2 runtime
  - **Linux** — `webkit2gtk-4.1`, `libayatana-appindicator3`, `librsvg2`, `build-essential`
  - **macOS** — Xcode Command Line Tools

```sh
git clone https://github.com/dmadan86/osstat.git
cd osstat
just setup     # install npm dependencies
just dev       # run the app with hot reload
just test      # Rust + frontend test suites
just ci        # everything a pull request must pass
```

`just` with no arguments lists every available target.

## Architecture

A Rust core with a webview front-end, built on [Tauri 2](https://tauri.app).
All privileged logic lives in Rust; the webview holds no OS access of its own.

```
crates/osstat-core/       portable domain types, traits and engines
crates/osstat-platform/   per-OS implementations, selected at compile time
src-tauri/                the desktop shell: commands, events, capabilities
ui/                       React + TypeScript front-end
docs/adr/                 the decisions behind all of the above
```

Every significant design decision is written down as an Architecture Decision
Record in [`docs/adr/`](docs/adr/). Start with
[ADR-001](docs/adr/ADR-001-application-framework.md) (why Tauri) and
[ADR-005](docs/adr/ADR-005-deletion-safety.md) (how deletion is kept safe).

## Contributing

Contributions are welcome — especially cleaning rules and LLM registry entries,
which need no Rust knowledge. See [CONTRIBUTING.md](CONTRIBUTING.md) for the
development setup, the commit convention, and the rule-authoring guide.

Security issues should go through [SECURITY.md](SECURITY.md), not a public issue.

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project, as defined in the Apache-2.0 license, shall be
dual-licensed as above, without any additional terms or conditions.
