# osstat task runner. Run `just` with no arguments to list every target.
#
# These are the same commands CI runs, so a green `just ci` locally means a
# green pull request. If you change a check here, change it in
# .github/workflows/ci.yml too.

set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

# Show the available targets.
default:
    @just --list

# Install every dependency the project needs.
setup:
    npm ci

# Run the desktop app against the Vite dev server with hot reload.
dev:
    npm run dev

# Build the release bundle (installers land in target/release/bundle).
build:
    npm run build

# Every test: Rust unit/integration tests plus the frontend suite.
test: test-rust test-ui

# Rust tests across the whole workspace.
test-rust:
    cargo test --workspace --all-features

# Frontend unit tests.
test-ui:
    npm run test --workspace @osstat/ui

# Criterion benchmarks. Performance gates are defined per milestone in
# ROADMAP.md; this target is a placeholder until M1 lands the first bench.
bench:
    cargo bench --workspace

# End-to-end suite (tauri-driver + WebdriverIO). Arrives in M5.
e2e:
    @echo "e2e suite lands in M5 - see ROADMAP.md"

# Every lint and format check, without modifying files.
lint: lint-rust lint-ui

lint-rust:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings

lint-ui:
    npm run lint --workspace @osstat/ui
    npm run typecheck --workspace @osstat/ui
    npm run format:check

# Apply every automatic fix.
fmt:
    cargo fmt --all
    npm run format

# Regenerate ui/src/bindings/ from the Rust IPC types (ADR-002).
# Run this after changing any type that crosses the IPC boundary.
bindings:
    cargo test --workspace --features ts-bindings

# Dependency vulnerability checks. Needs `cargo install cargo-audit`; CI uses
# the rustsec audit action instead, so this is for checking before you push.
audit:
    cargo audit
    npm audit --audit-level=high

# Everything a pull request must pass.
ci: lint test
