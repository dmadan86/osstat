# ADR-001: Application framework — Tauri 2

**Status:** Accepted

## Context

osstat needs one codebase for Windows, Linux and macOS with deep OS-level access:
process control, file deletion with elevation, and GPU queries. Candidates were
Electron, Tauri 2, Flutter Desktop, Qt/C++ and .NET MAUI.

## Decision

Use **Tauri 2** — a Rust backend with an OS-provided webview front-end.

## Rationale

- A Rust backend is ideal for system-level work: memory safety while calling OS
  APIs, and first-class crates for exactly this domain (`sysinfo`, `netstat2`,
  `nvml-wrapper`, `trash`).
- Binaries are roughly 10–20× smaller than Electron's, with far lower idle memory,
  because the OS webview is used instead of a bundled Chromium.
- The security model is strong: an explicit capability and permission system
  gates what the front-end may call, and commands are allow-listed.

Rejected alternatives:

- **Electron** — a 150+ MB footprint contradicts the product's identity. A
  cleaner that is itself bloated is not a credible cleaner.
- **Qt** — licensing friction (LGPL constraints or a commercial license), and
  C++ slows contribution velocity.
- **Flutter** — a weak desktop system-API story; every capability would be FFI
  anyway, which is the part we most want to be safe.

## Consequences

- The front-end runs in WebKitGTK on Linux, WebView2 on Windows and WKWebView on
  macOS. Rendering differs subtly between them, so end-to-end tests must run on
  each platform rather than one.
- All privileged logic lives in Rust. The webview never touches the OS directly;
  it can only call commands registered in the handler and permitted by
  `src-tauri/capabilities/`.
- We depend on the user having a webview runtime. On Windows the installer
  bootstraps WebView2; on Linux it is a packaging dependency.

## Implementation notes

The Tauri desktop template declares `staticlib` and `cdylib` crate types to
support mobile targets. Mobile is an explicit non-goal, so `src-tauri` builds
only an `rlib`, avoiding build time for artifacts that would never ship.
