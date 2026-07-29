#!/usr/bin/env bash
#
# Cold-start timing check (ROADMAP.md M5, "cold start under 2 s").
#
# Launches the release binary with `--measure-startup` under hyperfine. That
# flag makes osstat show its main window and exit immediately, so the
# process's wall-clock lifetime *is* the startup time hyperfine measures.
#
# What this number is, and what it is not
# ---------------------------------------
# It is honest about three limits rather than quietly rounding them away:
#
#  1. It starts at the first instruction of `osstat_lib::run`, so it excludes
#     the OS loader and dynamic-linking work that happens before `main`.
#     Nothing inside the process can observe that time. hyperfine's own
#     wall-clock figure *does* include it, which is why the hyperfine mean —
#     not the in-process number — is the one gated below.
#
#  2. It ends when the native window becomes visible, not when the front-end
#     paints inside it. The webview navigates to the bundled index.html
#     asynchronously afterwards. Closing that gap would mean the front-end
#     reporting back over IPC once mounted, adding a permanent round trip to
#     every real launch purely to serve a benchmark.
#
#  3. Only the first of the runs below is a true cold start. Every later run
#     finds the binary and its shared libraries already in the OS page cache,
#     and no portable way exists to drop that cache. The reported mean is
#     therefore a *floor* on a real first-ever launch, not an upper bound.
#
# Read the result as "the steady-state launch cost", and treat a regression in
# it as real even though the absolute figure is optimistic.

set -euo pipefail

# The under-2 s product goal, in seconds. Kept as a variable so a future
# tightening is one edit and shows up in the diff as a decision.
readonly TARGET_SECONDS=2.0

# Enough runs for the mean to mean something, few enough to stay bearable when
# each one starts and tears down a webview.
readonly RUNS=10

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if ! command -v hyperfine >/dev/null 2>&1; then
  echo "cold-start: hyperfine is not on PATH." >&2
  echo "            Install it with \`cargo install hyperfine\` (or your package manager)." >&2
  exit 127
fi

# OSSTAT_BIN lets a caller point at a bundled app instead — e.g. the macOS
# .app's inner executable, which is what an end user actually launches.
binary="${OSSTAT_BIN:-}"
if [ -z "$binary" ]; then
  case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN* | Windows_NT) binary="target/release/osstat.exe" ;;
    *) binary="target/release/osstat" ;;
  esac
fi

if [ ! -x "$binary" ]; then
  echo "cold-start: no release binary at $binary" >&2
  echo "            Build one first with \`just build\` (or \`cargo build --release\`)." >&2
  echo "            A debug build would measure the wrong thing entirely." >&2
  exit 1
fi

echo "cold-start: timing $binary over $RUNS runs (target: under ${TARGET_SECONDS}s)"
echo

json="$(mktemp)"
trap 'rm -f "$json"' EXIT

# --shell=none for two reasons. It keeps a shell's own spawn time out of a
# measurement whose whole subject is spawn time, and it sidesteps the fact
# that hyperfine's default shell on Windows is cmd.exe, which rejects the
# forward-slash path this script builds.
hyperfine \
  --shell=none \
  --runs "$RUNS" \
  --command-name "osstat --measure-startup" \
  --export-json "$json" \
  -- "$binary --measure-startup"

# hyperfine writes `"mean": <seconds>,` once per benchmarked command, and there
# is exactly one command here.
mean="$(grep -o '"mean":[[:space:]]*[0-9.eE+-]*' "$json" | head -1 | tr -d ' ' | cut -d: -f2)"

if [ -z "$mean" ]; then
  echo "cold-start: could not read a mean out of hyperfine's JSON export." >&2
  exit 1
fi

echo
# awk rather than bash arithmetic, which is integer-only.
awk -v mean="$mean" -v target="$TARGET_SECONDS" '
  BEGIN {
    printf "cold-start: mean %.3fs against a %.1fs target\n", mean, target
    if (mean > target) {
      printf "cold-start: OVER TARGET by %.3fs\n", mean - target
      exit 1
    }
    printf "cold-start: within target, %.3fs to spare\n", target - mean
  }
' || {
  echo "cold-start: see the three limits documented at the top of this script -" >&2
  echo "            a real first-ever launch is slower than this figure, so a" >&2
  echo "            failure here is a failure with room to spare." >&2
  exit 1
}

echo
echo "cold-start: the figure above is hyperfine's wall clock, which includes"
echo "            the OS loader. osstat also prints its own in-process number"
echo "            as 'osstat_cold_start_ms=', which hyperfine discards; run the"
echo "            binary directly to see it. On Windows a release build targets"
echo "            the windows subsystem, so that line reaches a pipe or a"
echo "            redirect but never an interactive console."
