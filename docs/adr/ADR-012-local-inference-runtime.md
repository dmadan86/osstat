# ADR-012: Local inference runtime

**Status:** Accepted

## Context

ADR-008 built an advisor: it weighs models against a machine and shows the
arithmetic. It cannot act on any of it. Answering "can I run this?" with a
coloured cell and then sending the user elsewhere is where that feature stops.

Running a model in osstat needs an inference engine. osstat's product goals are
an installer under ~20 MB, a cold start under 2 s, and idle RAM under 150 MB,
and it has never made a network request or executed a binary it did not ship.
Every option here is in tension with at least one of those.

## Decision

**Fetch a prebuilt `llama-server` on demand**, from the `ggml-org/llama.cpp`
GitHub releases, rather than bundling it or linking llama.cpp in-process.

**Select the backend from the ADR-008 probe**, as a pure function of target OS,
architecture and what the probe found. Defaults: Metal on macOS, Vulkan on
Windows and Linux, CPU where no GPU build exists.

**CUDA is an opt-in, not an automatic selection.** It is offered on Windows x64
when NVML reports an NVIDIA driver, with its download size stated, and taken
only if the user accepts.

**Verify against a checksum pinned in this repository.** `runtimes.json` maps
each artifact to a filename, SHA256 and size for one upstream tag. A mismatch
aborts: no retry, no fallback, no override.

## Rationale

**Why not bundle.** The Vulkan builds are 26–34 MB and the Windows CUDA builds
538–642 MB once the separate CUDA runtime archive is counted. Against a ~20 MB
installer budget, bundling means either shipping one backend per platform — and
telling NVIDIA owners to do without — or shipping a matrix of them.

**Why not link llama.cpp in-process.** `llama-cpp-2` builds llama.cpp from
source, which puts a C++ and CUDA toolchain on all three CI runners, fixes the
GPU backend at compile time, and makes an inference OOM a crash of the whole
app — taking down the tray and the sampler with it. A monitoring tool that dies
because a model was too big has failed at its actual job.

**Why Vulkan by default.** Upstream publishes nine backend variants for
Windows and Linux between them. Vulkan works on every desktop vendor and
collapses that to one row per platform. Each additional backend is a row no CI
runner can exercise and nobody on the project can test by hand.

**Why CUDA is opt-in.** CUDA is genuinely faster on NVIDIA hardware, and the
first draft of this design selected it automatically when the driver was
present. The published sizes changed that:

| Backend                      | Download |
| ---------------------------- | -------- |
| `win-vulkan-x64`             | 34 MB    |
| `win-cuda-13.3-x64` + cudart | 538 MB   |
| `win-cuda-12.4-x64` + cudart | 642 MB   |

Starting a 642 MB download because a graphics card was detected is not a
defensible default. Showing both with their sizes is the same argument ADR-008
makes for the explanation drawer: the user can check the decision, and change
it.

**Why the checksum is pinned rather than fetched.** GitHub's release API returns
a digest per asset, and using it would keep osstat permanently current. It would
also verify almost nothing: the digest and the binary arrive from the same
origin, so it proves the transfer was not corrupted, not that the file is the
one anybody reviewed. A hash compiled into osstat means compromising a release
cannot change what osstat will execute, and changing it is a pull request whose
diff a person can read.

The cost is that the pin ages — upstream ships builds close to daily; the tag
moved twice while this design was being written. That is the intended trade. It
is also what makes "works offline once acquired" a property that can be tested
rather than hoped for.

## Consequences

- **osstat now executes a binary it downloaded.** SECURITY.md gains a fifth
  threat for this, with the controls: pinned hash, refusal on mismatch, never
  elevated, arguments as a vector and never through a shell, app-private
  directory, and archive entries refused if they would escape it.
- **osstat now makes network requests.** SECURITY.md's privacy section already
  allows this — "no network requests except ones you explicitly trigger" — and
  that phrasing is a constraint, not a loophole: no background polling, no
  prefetch, and no silent check for a newer upstream build.
- **The CSP is untouched.** All egress is in Rust; the webview never issues an
  HTTP request, so a compromised webview still cannot exfiltrate (threat 3).
- **The pin must be refreshed deliberately.** Procedure below.
- **Linux NVIDIA users get Vulkan, not CUDA**, because upstream publishes no
  Linux CUDA artifact at all. Windows on ARM gets CPU only, because upstream
  publishes no Vulkan build for it. Both are reported honestly rather than
  presented as osstat's choice.
- **CI does not download a runtime or run inference.** Both are slow and
  network-flaky. Tests are hermetic — a fixture HTTP server, a stub archive —
  and real acquisition is a manual gate whose verified platforms are named
  rather than implied by a green build.

## Refreshing the pin

`runtimes.json` is data, reviewed as data. To move to a newer upstream build:

1. Read the release manifest for the new tag, including the per-asset `digest`
   and `size` fields:
   `curl -s https://api.github.com/repos/ggml-org/llama.cpp/releases/latest`
2. Update `upstreamTag`, and every `file`, `sha256` and `sizeBytes` — including
   the `companion` entries for the two Windows CUDA artifacts.
3. Run `cargo test -p osstat-inference`. The schema check, the tag-in-filename
   check and the two selection/manifest drift checks all run there.
4. Open a pull request. The diff is a set of hashes; that is the review.

If upstream renames or drops an artifact, the drift tests fail rather than the
app failing on a user's machine. Adding a backend is one `runtimes.json` entry
and one arm in `Backend::artifact_id`.
