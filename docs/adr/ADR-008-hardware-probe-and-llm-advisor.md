# ADR-008: Hardware probe and LLM runnability advisor

**Status:** Accepted

## Context

"Can my machine run this model?" is a question people currently answer by
downloading a 20 GB file and finding out. Answering it accurately requires
knowing VRAM, system RAM and CPU, and knowing how a given model and quantization
consume them.

## Decision

**GPU and VRAM detection**, in order of preference:

1. NVIDIA via `nvml-wrapper` — exact VRAM, including free and used
2. AMD, Intel and Apple via `wgpu` adapter enumeration — name plus memory heuristics
3. macOS unified memory reported _as_ unified memory, with VRAM derived from
   system RAM

**Model registry:** a versioned JSON file, updatable independently of releases,
listing popular open models with parameter counts, file sizes per quantization
(Q4_K_M, Q5, Q8, FP16) and minimum context memory footprint.

**Runnability arithmetic**, documented in-app rather than hidden:

```
required_memory ≈ model_file_size × 1.1 + kv_cache(context_length, layers, heads)
```

Each model × quantization is classified as: **fits in VRAM**, **fits with CPU
offload** (with an estimate of layers offloaded and a rough tokens-per-second
tier), or **will not run**.

The advisor also recommends runtimes (llama.cpp, Ollama, LM Studio) and detects
whether Ollama is installed, offering a "pull this model" deep link when it is.

## Rationale

Showing the arithmetic is the point. A verdict a user cannot check is a verdict
they cannot trust, and this one depends on assumptions (context length, offload
behaviour) that vary by runtime. An expandable explanation turns a black box into
something a user can argue with — and correct.

The registry as a separate JSON file means new models can ship without a release,
and it is a low-barrier contribution surface for the community.

## Amendment, 2026-08-01: two more sources, and a second memory pool

The three sources above leave every AMD and Intel adapter reporting no video
memory at all, and none of them knows about the shared pool — the system memory
a GPU borrows when its own runs out, which Task Manager has shown users for
years. Two sources are added:

4. **Windows via DXGI and the `GPU Adapter Memory` performance counters**
   (`GpuSource::Dxgi`) — `DXGI_ADAPTER_DESC1` for both totals,
   `PdhAddEnglishCounterW` against the counter set Task Manager reads for live
   usage. Measured.
5. **Linux via amdgpu's DRM sysfs files** (`GpuSource::DrmSysfs`) —
   `mem_info_vram_*` and `mem_info_gtt_*`. Measured, amdgpu only.

`IDXGIAdapter3::QueryVideoMemoryInfo` is deliberately not used. It looks exactly
right and is not: Microsoft documents its `CurrentUsage` as "the application's
current video memory usage" — a budgeting API for a process to manage its own
footprint, not a description of the machine. Reading it would put osstat's own
footprint on screen under a label claiming to describe the machine. Live usage
comes from the performance counters instead.

macOS gains nothing, and that is the correct outcome rather than a gap. Apple
Silicon has one memory pool, not two; the existing unified-memory path already
reports it. Live GPU usage there is reachable only through the undocumented
`PerformanceStatistics` dictionary on the `IOAccelerator` IOKit service, which
is a poor foundation for a figure labelled "measured".

**Provenance is now per-figure.** An NVIDIA card on Windows carries two sources
at once — NVML measures the dedicated pool, only the counters measure the
shared one — so `GpuDevice::shared_source` records the second rather than
letting one figure borrow the other's credibility. The front end decides the
"estimated" label per pool.

**A reported zero means unknown, not zero.** DXGI returns
`DedicatedVideoMemory: 0` for an adapter with no VRAM of its own, and the
amdgpu sysfs files hit the same case. Both readers normalise a `0` to `None`
rather than `Some(0)` — "no VRAM of its own" and "the platform did not report a
figure" are different claims, and collapsing them would make a meter divide by
zero the moment either happens for real. `osstat_platform::gpu_memory::non_zero`
is the single point where the rule is enforced.

**Shared memory is excluded from the runnability arithmetic.** It is system RAM
across PCIe at roughly a tenth of VRAM's bandwidth. The calculator already
weighs system memory separately and answers "fits with CPU offload"; counting
the shared pool as VRAM would turn that honest verdict into a false "fits in
VRAM" one — the outcome the previous paragraph names as the most damaging thing
this feature could do. Two regression tests in `calculator.rs` hold the line:
`shared_memory_never_counts_as_vram` and
`a_shared_pool_does_not_inflate_a_real_vram_figure`.

**Module placement.** These per-OS implementations live in `osstat-platform`,
complying with ADR-003 rather than deviating from it. The original plan put
them in `osstat-llm` beside the calculator, but that crate carries
`#![forbid(unsafe_code)]`, which — unlike `deny` — cannot be lifted by a scoped
`#[allow]`, and DXGI and PDH access require raw FFI. Only `osstat-platform` uses
`#![deny(unsafe_code)]`, deliberately, because ADR-003 already puts every
per-OS implementation there. The new code follows the existing
`windows.rs` / `linux.rs` / `macos.rs` split: each gained an `adapter_memory()`
function beside the capabilities already there. The portable work splits across
two files for two different jobs — parsing and normalising raw platform data
(`osstat-platform/src/gpu_memory.rs`: the counter-instance-name parser, the
sysfs byte-count parser, `non_zero`) and merging those readings into the
`GpuDevice` list the GPU probe already built (`osstat-llm/src/probe/adapter_memory.rs`)
— both compiled and tested on every platform, same as the rest of their crates.

## Consequences

- **These are estimates and must be labelled as such.** Real throughput depends
  on the runtime, quantization implementation, memory bandwidth and thermal
  behaviour. Presenting a heuristic as a measurement would be the most damaging
  thing this feature could do to the project's credibility.
- The calculator is pure functions with no I/O, so it can be tested
  exhaustively. The M4 gate requires 100% branch coverage on it.
- Probing must degrade gracefully when there is no GPU at all. CI runners have
  none, which makes that path continuously tested rather than an afterthought.
- `wgpu` is a heavy dependency for what is ultimately adapter enumeration. Its
  effect on the ~20 MB installer budget should be measured when M4 lands, and a
  lighter platform-specific probe considered if it proves significant.
