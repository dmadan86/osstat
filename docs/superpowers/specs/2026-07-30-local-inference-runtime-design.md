# Local inference, part 1: acquiring the llama.cpp runtime

**Date:** 2026-07-30
**Status:** Approved, not yet implemented.
**Supersedes:** nothing. **Depends on:** ADR-002, ADR-003, ADR-008, ADR-010.
**Obliges:** a new ADR-012, and a fifth threat in SECURITY.md. See §8.

## Context

M4 shipped an advisor. It reads the machine, weighs fifteen models at four
quantizations against it, and shows the arithmetic behind every verdict. What it
cannot do is let you act on any of it: the answer to "can I run this?" is a
coloured cell, and the next step is to leave osstat and go elsewhere.

The request is to close that gap — pick a model in the UI, run it, talk to it.

That is not a feature. It is five subsystems: acquiring a runtime, acquiring
weights, supervising an inference process, storing conversations, and rendering a
chat. Specifying all five at once produces a document nobody reviews and a plan
that is stale by its third step, so the work is split into four sub-projects, each
with its own spec, plan and implementation cycle:

|          | Sub-project             | Delivers                                                                                    |
| -------- | ----------------------- | ------------------------------------------------------------------------------------------- |
| **M4.1** | **Runtime acquisition** | **The llama.cpp server binary, chosen for this machine, verified, and on disk. This spec.** |
| M4.2     | Model acquisition       | Hugging Face search, resumable GGUF download, local file import, header parsing             |
| M4.3     | Inference session       | Spawn, supervise, stream, cancel, tear down                                                 |
| M4.4     | Chat                    | Conversation store, streaming UI, markdown, controls, live tok/s and context meters         |

M4.1 is first because everything downstream needs a binary to talk to, and
because it carries the security decisions. It is also the sub-project with no
visible payoff — at the end of it osstat can fetch and verify a runtime and do
nothing with it. That is expected, and is why it is specified rather than
improvised on the way to something more interesting.

## Why this belongs in osstat at all

Two connections make this coherent rather than a chat app bolted to a system
monitor:

**The advisor's output is the executor's input.** `calculator.rs` already
computes `verdict.gpu_layers`. That figure is precisely llama-server's
`--n-gpu-layers`. The arithmetic osstat shows you is the arithmetic it then acts
on.

**Running the model measures what the advisor estimates.** ADR-008 states that
presenting a heuristic as a measurement is the most damaging thing this feature
could do, and it labels the speed tier an estimate for exactly that reason.
llama-server returns real timings on every response. The advisor's guess and the
machine's actual throughput can finally be shown side by side — which is also the
roadmap's "quick benchmark to refine the LLM advisor's tokens-per-second
estimates", arriving as a by-product rather than a separate feature.

## Goals

1. osstat obtains a llama.cpp server binary appropriate to this machine, on
   Windows, Linux and macOS, on x64 and arm64.
2. It never executes a binary whose contents it has not verified.
3. The installer stays within its ~20 MB budget, and a user who never opens the
   LLM page pays nothing — no download, no dependency loaded, no startup cost.
4. Every byte that moves over the network moves because a user asked for it.
5. What osstat downloaded is visible and deletable from inside osstat.

## Non-goals

- **Running a model.** M4.3. This sub-project ends with a verified binary on
  disk.
- **Downloading weights.** M4.2. Runtimes and models are different artifacts with
  different sizes, sources and trust properties.
- **Bundling the runtime in the installer.** Rejected in §1.
- **Building llama.cpp from source.** No C++ toolchain enters this project's CI.
- **Tracking upstream automatically.** The pinned build changes in a reviewed
  pull request, never at runtime. See §3.
- **Supporting every backend upstream publishes.** OpenVINO, SYCL, s390x and
  Adreno are real artifacts that this design deliberately does not select. See
  §2.
- **Using a llama-server the user already has.** Someone who built llama.cpp
  themselves cannot benefit from this sub-project, and that is deliberate: osstat
  cannot verify a binary it did not fetch, and adding an unverified path here
  would hollow out §3 on the same day it is written. It is a reasonable thing to
  want, it is revisitable once M4.3 exists, and it needs its own decision about
  how an unverified runtime is labelled — not a quiet exception to this one.

---

## 1. Decisions taken during design

Recorded because each closed off an option that will look attractive again later.

| Decision                               | Rejected alternative                                   | Why                                                                                                                                                                                   |
| -------------------------------------- | ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Fetch the runtime on demand            | Bundle it in the installer                             | A Vulkan or CUDA build is 50–200 MB against a ~20 MB installer budget, and bundling means shipping one backend per platform or a matrix of them.                                      |
| Fetch the runtime on demand            | Link llama.cpp in-process (`llama-cpp-2`)              | Needs a C++/CUDA toolchain on all three CI runners, fixes the GPU backend at compile time, and an inference OOM would take the tray and the sampler down with it.                     |
| Vulkan by default on Windows and Linux | CUDA/ROCm/SYCL per vendor                              | Collapses a nine-way matrix to two and works on every vendor. The cost is named in §2 rather than hidden.                                                                             |
| Checksums pinned in the repo           | Digests read from the GitHub API at download time      | The API digest arrives from the same origin as the binary, so it proves transport, not authenticity. A pinned hash means compromising the release cannot change what osstat will run. |
| A new `osstat-inference` crate         | Extending `osstat-llm`                                 | `osstat-llm` is cheap and near-pure, and `src-tauri` depends on it. Pulling an HTTP client and process supervision into it taxes cold start for every user who never opens the page.  |
| Selection is a pure function           | Selection queries the network                          | Makes the whole of §2 exhaustively table-testable with no I/O, the shape `calculator.rs` already proved.                                                                              |
| All network egress in Rust             | The webview calling Hugging Face or localhost directly | SECURITY.md threat 3. The CSP is a control, and a compromised webview that cannot reach the network cannot exfiltrate.                                                                |

## 2. Backend selection

Selection is a pure function of three inputs osstat already has — target OS,
target architecture, and the ADR-008 hardware probe — and produces an artifact
identifier. No I/O, so it is exhaustively testable.

Upstream's published artifacts for release `b10192` reduce to this table. Every
row names a real published asset:

| OS      | Arch  | GPU                      | Artifact                              |
| ------- | ----- | ------------------------ | ------------------------------------- |
| macOS   | arm64 | any                      | `macos-arm64`                         |
| macOS   | x64   | any                      | `macos-x64`                           |
| Windows | x64   | NVIDIA, CUDA 13.x driver | `win-cuda-13.3-x64` **+ cudart 13.3** |
| Windows | x64   | NVIDIA, CUDA 12.x driver | `win-cuda-12.4-x64` **+ cudart 12.4** |
| Windows | x64   | AMD, Intel, other        | `win-vulkan-x64`                      |
| Windows | x64   | none                     | `win-cpu-x64`                         |
| Windows | arm64 | any                      | `win-cpu-arm64`                       |
| Linux   | x64   | any GPU                  | `ubuntu-vulkan-x64`                   |
| Linux   | x64   | none                     | `ubuntu-x64`                          |
| Linux   | arm64 | any GPU                  | `ubuntu-vulkan-arm64`                 |
| Linux   | arm64 | none                     | `ubuntu-arm64`                        |

Three properties of upstream's matrix drive this, and each contradicts an
assumption that would otherwise be natural:

**There is no Linux CUDA build.** On Linux with an NVIDIA card the choice is
Vulkan or CPU. Any design assuming "NVIDIA means CUDA" is wrong on one of the
three supported platforms.

**Windows CUDA needs a second artifact.** `cudart-llama-bin-win-cuda-12.4-x64.zip`
ships the CUDA runtime DLLs separately. Two downloads, or the server will not
start. This is the only row in the table that is not a single file, and it is the
one most likely to be got wrong.

**Windows on ARM has no Vulkan build.** Only `win-cpu-arm64` and an
Adreno-specific OpenCL build exist. Windows/arm64 is therefore CPU-only. This is
an upstream limitation and osstat reports it as one — the UI says the machine has
a GPU that this runtime cannot use here, rather than silently reporting no GPU.

macOS needs no variant selection at all: Metal is built into both macOS builds.

Upstream also publishes OpenVINO, SYCL, ROCm, Adreno/OpenCL and s390x builds.
None is selected. Each would add a row that no CI runner can exercise and that
nobody on the project can test by hand, in exchange for a speedup over Vulkan on
hardware the table already covers. Adding one later is a `runtimes.json` entry
and a selection branch, both cheap; carrying five untested ones from the start is
not.

The CUDA 12.4-versus-13.3 choice is answerable from data osstat already collects.
NVML reports the driver's CUDA version, so the ADR-008 probe extends to backend
selection rather than requiring a second probe.

**The cost of Vulkan-by-default, stated plainly:** CUDA is meaningfully faster
than Vulkan on NVIDIA hardware. Windows NVIDIA users get the fast path because
upstream builds it; Linux NVIDIA users do not, because upstream does not. osstat
should say which backend it selected and why, in the same spirit as the advisor's
explanation drawer — a chosen backend the user cannot see is a performance
difference they cannot explain.

## 3. Pinning and verification

A `runtimes.json` in the repository, beside `models.json` and validated against
its own schema in a test, maps each artifact identifier to a filename, a SHA256
and a byte size, all pinned to one upstream release tag.

The file is data, reviewed like data. Bumping the runtime is a pull request whose
diff is a set of hashes — visible, reviewable, revertable — rather than a decision
taken at runtime on the user's machine.

**Verification is refuse-by-default.** osstat computes the SHA256 of what it
downloaded and compares it to the pinned value before extracting anything and
before setting any executable bit. A mismatch stops the operation and reports it
as what it is. It never retries, never falls back to a different artifact, and
never offers to continue anyway: a hash mismatch is a security event, not a
transient network error, and the two must not share a code path.

The trade-off: upstream ships builds close to daily, so osstat's pin ages between
releases. For a utility whose stated values are predictability and auditability,
a known-good pinned build beats an unverifiable current one. It is also what
makes "works offline once acquired" a property that can be tested.

## 4. Where it lives

Per AGENTS.md's placement rules and ADR-003:

**`crates/osstat-inference`** — new workspace member. Owns selection, the pinned
manifest, download, verification, extraction, and the on-disk layout. Depends on
`osstat-llm` for the probe result. `osstat-llm` is not modified: it stays the
advisor, pure and cheap.

**`crates/osstat-platform`** — gains the OS-specific pieces behind traits: setting
the executable bit, and (for M4.3) the orphan-prevention primitive. Nothing
OS-specific lives in `osstat-inference`.

**`src-tauri`** — IPC adapters only. Commands to report runtime status, start an
acquisition, report its progress, and delete what was downloaded. No logic.

**`ui`** — presentation only, over IPC and events. It never issues an HTTP
request. The CSP in `tauri.conf.json` is not modified by this work, and any
proposal to modify it is out of scope by definition.

**On disk:** the Tauri app-data directory, under a subdirectory keyed by upstream
tag and artifact identifier, so two pinned versions can coexist during an upgrade
and neither is clobbered in place. Because osstat is also a disk cleaner, what it
downloaded is listed with its size and is deletable from Settings. Leaving
multi-hundred-megabyte artifacts around that osstat does not admit to would be
the exact behaviour this project exists to complain about.

**New dependencies:** fewer than the framing suggests. This is the first code in
osstat to make a network request, but it is not the first network stack in the
tree — `reqwest 0.13.4` and `tokio 1.53.1` are already in `Cargo.lock`, reached
through Tauri. (That path is how `webpki-root-certs` came to need an exception in
`deny.toml`.) The work uses crates that are already compiled and already through
the ADR-010 licence gate, rather than introducing a stack. Extraction needs a zip
and a tar/gzip reader, which are genuinely new and small.

**Cost when unused:** acquisition is triggered by the user, and the crate does no
work at startup. A user who never opens the LLM page downloads nothing and pays
no startup cost.

## 5. Data flow

```
User clicks "Set up the runtime"
  → IPC: acquire_runtime
    → select(os, arch, probe) → artifact id          [pure, §2]
    → look up filename + sha256 + size in runtimes.json
    → check free disk space against the pinned size, doubled
      (the archive and its extracted contents coexist)
    → download to a temporary file, emitting runtime:progress
    → SHA256 the temporary file
    → mismatch? stop, report, leave nothing behind   [§3]
    → extract into <appdata>/runtimes/<tag>/<artifact>/
    → set the executable bit (osstat-platform)
    → verify llama-server exists where expected
    → emit runtime:ready
```

Events follow the existing `metrics:tick` naming: `runtime:progress`,
`runtime:ready`, `runtime:failed`.

Downloading to a temporary file and moving it into place only after verification
means an interrupted or corrupted download can never be mistaken for a usable
runtime, and no partial state survives a failure.

## 6. Failure modes

Each is a distinct variant of a typed error with its own message. No `unwrap` or
`expect` outside tests, per AGENTS.md.

| Failure                                 | Behaviour                                                                                                                                                                             |
| --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| No network                              | Report plainly. Retry is a button, not automatic.                                                                                                                                     |
| This platform has no artifact           | Report the platform and that the runtime does not build for it. Do not substitute a different architecture.                                                                           |
| Not enough disk space                   | Refuse before downloading, naming the space needed and available. The requirement is roughly twice the artifact size, because the archive and its extracted contents briefly coexist. |
| Download interrupted                    | Temporary file discarded. Resume is M4.2's problem, where files are large enough to warrant it.                                                                                       |
| **Checksum mismatch**                   | **Stop. Report as a verification failure. Delete the temporary file. Never retry, never fall back, never offer to continue.**                                                         |
| Extraction failure                      | Report; remove the partial directory.                                                                                                                                                 |
| `llama-server` missing after extraction | Treat as a verification failure — the archive was not what was expected.                                                                                                              |
| Executable bit cannot be set            | Report with the path, so a noexec mount or a policy is diagnosable rather than mysterious.                                                                                            |

## 7. Testing

AGENTS.md requires tests in the same change, and most of this is pure, so most of
it is cheap to test.

**Pure, exhaustive:**

- Selection over every combination of OS, arch and probe result, including no
  GPU, an unreported-VRAM GPU, and each vendor. This is `calculator.rs`'s shape,
  and the same standard applies.
- `runtimes.json` validated against its schema, as `models.json` already is.
- **Every platform in the CI matrix resolves to some artifact.** This is the
  three-platform requirement made executable rather than assumed.
- Every artifact identifier reachable from selection exists in `runtimes.json`,
  and every entry is reachable — neither can drift from the other unnoticed.

**Hermetic, with fixtures:**

- Download against a local fixture server: success, mid-transfer disconnect,
  404, and a body whose hash does not match.
- **Checksum refusal is the single most important test in this sub-project.** It
  asserts that a mismatched body is never extracted, never made executable, and
  leaves nothing behind.
- Extraction into a temporary tree, asserting layout and that `llama-server` is
  present and executable.
- Disk-space refusal, using a pinned size larger than any plausible free space.

**Explicitly not in CI:** downloading a real 200 MB artifact, and running real
inference. Both are slow and network-flaky, and neither belongs on a pull
request. Real acquisition is a manual gate, and per AGENTS.md the milestone will
record which platforms were actually verified by hand rather than letting a green
CI imply it.

CI runners have no GPU, so the no-GPU selection path is continuously exercised —
the same property ADR-008 notes for the probe.

## 8. Documents this obliges

**ADR-012 — local inference runtime.** Records fetch-on-demand over bundling and
over in-process linking, Vulkan-by-default, and pinned checksums over API
digests. This is the class of decision the project records, and a future
contributor will otherwise re-litigate all three.

**SECURITY.md — a fifth threat.** The current model names four, and none covers
executing a binary that osstat downloaded. The new entry should state the
controls: a pinned hash compiled into osstat rather than fetched alongside the
artifact, refusal on mismatch, never running elevated, arguments passed as a
vector and never through a shell, and an app-private directory.

The Privacy section needs no change and should be re-read as a constraint:
osstat "makes no network requests except ones you explicitly trigger." That
already licenses this feature and simultaneously forbids background upstream
polling, prefetch and silent update checks. This design has none, and that is a
requirement, not a coincidence.

## 9. What "done" means

- [ ] `osstat-inference` exists as a workspace member with selection, manifest,
      download, verification and extraction, each with tests in the same change.
- [ ] `runtimes.json` and its schema are committed and schema-validated in a test.
- [ ] Selection resolves for every platform in the CI matrix, proven by test.
- [ ] A checksum mismatch is proven to leave nothing executable behind.
- [ ] IPC commands and events are wired, with generated bindings up to date
      (`just bindings`).
- [ ] Settings lists what was downloaded, with size, and can delete it.
- [ ] ADR-012 written; SECURITY.md threat 5 added.
- [ ] `just ci` green on Windows, Linux and macOS.
- [ ] Manual acquisition verified on at least one real machine per platform, with
      the platforms actually tested named in the milestone note.
