# Shared GPU memory in the Overview

**Date:** 2026-08-01
**Status:** Designed.
**Depends on:** ADR-003, ADR-008.
**Amends:** ADR-008, which names three GPU sources and does not anticipate a
second memory pool per device.
**Touches:** `osstat-core/src/gpu.rs`, `osstat-llm/src/probe.rs`,
`osstat-llm/src/calculator.rs` (by consequence, not by edit),
`ui/src/pages/Overview.tsx`.

## Context

The Overview's GPU section reports one memory figure per device, `vramTotal`,
and on most Windows machines it reports nothing at all:

> Video memory is not reported by this source (Vulkan).

That message is honest — ADR-008 chose it over a fabricated number — but it is
also the whole answer for every AMD card, every Intel card, and every integrated
GPU on Windows and Linux. NVML is the only source that measures anything, and
NVML is NVIDIA-only.

Meanwhile Windows Task Manager shows every user two figures per adapter:
**dedicated GPU memory** (the card's own VRAM) and **shared GPU memory** (system
RAM the GPU may borrow when its own runs out). osstat shows neither on most
hardware. This spec closes both gaps with one probe.

## Goals

1. Report shared GPU memory — total and in use — beside dedicated VRAM, on every
   platform that can honestly supply it.
2. Give AMD and Intel adapters a real dedicated VRAM figure instead of an
   apology.
3. Never present one source's provenance as another's, per ADR-008.
4. Show one meter where there is one pool, and two where there are two — never a
   second bar reading `0 GB of 0 GB`.

## Non-goals

- **Feeding shared memory into the LLM advisor.** See §5. This is the single
  most important constraint in this document.
- **Per-process GPU memory attribution.** Windows exposes it (`GPU Process
Memory`), and it belongs to a future process-table column, not here.
- **Charting GPU memory over time.** The section has no chart today and does not
  gain one.
- **macOS usage figures.** See §4.3.
- **Replacing NVML.** Where NVML measures a figure, NVML keeps measuring it.

---

## 1. Decisions taken during design

### 1.1 PDH performance counters, not `IDXGIAdapter3::QueryVideoMemoryInfo`

DXGI has an API that looks exactly right and is not. Microsoft documents
`DXGI_QUERY_VIDEO_MEMORY_INFO::CurrentUsage` as:

> Specifies the **application's** current video memory usage, in bytes.

It is a budgeting API for a process to manage its own footprint. Reading it
would put osstat's own GPU memory on screen under a label claiming to describe
the machine. `Budget` has the same problem — it is the OS's suggestion to _this_
application, not a system total.

Task Manager reads Windows performance counters, and so will osstat. Verified on
the development machine:

```
CounterSetName : GPU Adapter Memory
Counters       : \GPU Adapter Memory(*)\Shared Usage
                 \GPU Adapter Memory(*)\Dedicated Usage
                 \GPU Adapter Memory(*)\Total Committed
```

```
luid_0x00000000_0x0001068c_phys_0   shared=134,569,984  dedicated=1,966,735,360
luid_0x00000000_0x00014292_phys_0   shared=      8,192  dedicated=            0
```

Two adapters, system-wide figures, instances keyed by adapter LUID. `Total
Committed` is not read: it is the sum of the other two, and deriving a third
number the UI does not show is work with no consumer.

### 1.2 Counters are added by their English names

PDH resolves counter paths against the installed display language. `PdhAddCounterW`
with `"\GPU Adapter Memory(*)\Shared Usage"` returns `PDH_CSTATUS_NO_OBJECT` on a
German or Japanese Windows. `PdhAddEnglishCounterW` takes the locale-invariant
name and is the correct call. This is the kind of defect that passes every test
on the developer's machine and fails for a whole class of users, so it is a
decision recorded here rather than a detail left to implementation.

### 1.3 Two sources on one device, stated as two

On the commonest configuration this feature will meet — an NVIDIA card on
Windows — the two memory pools come from two different places. NVML measures
dedicated VRAM, utilisation and temperature. Only PDH measures the shared pool.
A single `source` field cannot describe that, and collapsing it would mean
labelling one figure with another figure's provenance, which is precisely what
ADR-008 exists to prevent. Hence `shared_source` in §2.

### 1.4 A wgpu-identified adapter whose memory DXGI measured is a DXGI device

`GpuDevice::source` is documented as "which probe produced this device, and how
far to trust its numbers" — it is a trust label on the figures, and the
front-end uses `is_measured()` to decide whether to append "— estimated".

When DXGI supplies real memory totals for an adapter that `wgpu` merely named,
leaving `source: Wgpu` would stamp "estimated" on a measured figure. The device
is therefore promoted to `source: Dxgi`. It was still found by adapter
enumeration; that is provenance of the _name_, which no user reads a trust
judgement from.

### 1.5 The dedicated figure keeps its best available source

DXGI reports `DedicatedVideoMemory` for NVIDIA adapters too, so using DXGI
uniformly for both pools would be simpler. It is not done. NVML is the vendor's
own instrumentation and is authoritative for NVIDIA hardware; silently replacing
a working measured path with a second-hand one is a regression on the source
ADR-008 trusts most. DXGI fills gaps; it does not displace.

---

## 2. Data model

Three additive fields in `osstat-core/src/gpu.rs`. No existing field changes
meaning, so no existing consumer changes behaviour except where §5 says so.

```rust
pub struct GpuDevice {
    // … index, name, vendor, backend, kind unchanged
    pub vram_total: Option<u64>,
    /// System memory this GPU may borrow when its own is exhausted.
    ///
    /// `None` where the platform has no such pool — Apple unified memory is one
    /// pool, not two — or where it has one that cannot be measured.
    pub shared_total: Option<u64>,
    pub source: GpuSource,
    /// Where the shared figures came from, when that is not `source`.
    ///
    /// `None` when there is no shared figure. See §1.3.
    pub shared_source: Option<GpuSource>,
}

pub struct GpuSample {
    // … index, utilisation, vram_used, temperature_c unchanged
    /// Shared system memory in use by this GPU, in bytes.
    pub shared_used: Option<u64>,
}
```

`GpuSource` gains two variants:

```rust
/// DXGI adapter description plus Windows GPU performance counters: measured,
/// system-wide, and the same source Task Manager reads.
Dxgi,
/// The kernel's DRM sysfs interface, `/sys/class/drm/card*/device/mem_info_*`:
/// measured, amdgpu only.
DrmSysfs,
```

Both return `true` from `is_measured()`. The existing test
`only_nvml_counts_as_measured` is renamed and extended rather than deleted — the
property it guards is that `Wgpu` and `UnifiedMemory` are _not_ measurements,
and that property is unchanged.

`GpuSample::unmeasured` gains `shared_used: None`, and `has_measurements()`
gains `|| self.shared_used.is_some()` — a device whose only readable figure is
its shared pool still has something worth showing.

### 2.1 Invariants

- `shared_used` is `Some` only where `shared_total` is `Some`. A usage figure
  without a denominator cannot be drawn as a meter and would be shown as a bare
  byte count, which is not what this feature is for.
- `shared_source` is `Some` if and only if `shared_total` is `Some`.
- No source ever synthesises `shared_total` from system RAM. Halving the
  machine's memory is what Windows _happens_ to do today, and a heuristic that
  resembles the true answer is the specific failure ADR-008 names as most
  damaging — it is indistinguishable from a measurement until it is wrong.
- **A reported zero becomes `None`, not `Some(0)`.** DXGI returns
  `DedicatedVideoMemory: 0` for an integrated adapter that has no VRAM of its
  own. `Some(0)` and `None` would render identically as "no meter" today, but
  they mean different things — "measured, and there is none" against "not
  known" — and the difference would surface the moment anything divides by the
  total. Every source normalises `0` to `None` at the point of reading, for
  both pools, so no consumer downstream has to remember to.

---

## 3. Where the per-OS code lives

This is genuinely per-OS code, and ADR-003 puts per-OS code in
`osstat-platform`, as exactly one `mod imp` selected by `cfg(target_os)`. But
the GPU probe is in `osstat-llm`, which does not depend on `osstat-platform`.
Three options were weighed:

| Option                                  | Cost                                                                                                                                                  |
| --------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| Move the probe into `osstat-platform`   | Drags `wgpu` — ADR-008 already calls it "a heavy dependency" with an unmeasured installer cost — into the lean crate every other capability lives in. |
| Depend `osstat-llm` → `osstat-platform` | Acyclic and legal, but pulls `sysinfo` and `netstat2` into a crate that needs neither, to reach three small functions.                                |
| **A per-OS module inside `osstat-llm`** | Deviates from the letter of ADR-003.                                                                                                                  |

The third is chosen, because the crate already does exactly this and ADR-008
already sanctioned it: `probe/nvml.rs` is `#[cfg(not(target_os = "macos"))]`
with a macOS stub whose comment reads "keeps the probe's control flow identical
on all three platforms rather than scattering `cfg` through it." A sibling
`probe/adapter_memory/` with one module per OS behind a uniform free function is
the same pattern in the same crate.

```
crates/osstat-llm/src/probe/
  adapter_memory.rs          // mod imp selection + the portable merge
  adapter_memory/windows.rs  // DXGI totals + PDH usage
  adapter_memory/linux.rs    // amdgpu sysfs
  adapter_memory/macos.rs    // returns nothing; see §4.3
```

Each per-OS module exposes one function with an identical signature:

```rust
pub(crate) fn adapter_memory() -> Vec<AdapterMemory>;
```

`AdapterMemory` is crate-internal — a PCI vendor/device pair, an optional
adapter name, and the four figures. It never crosses the IPC boundary; the merge
in `adapter_memory.rs` folds it into `GpuDevice` and `GpuSample`, which do.

ADR-008 gains an amendment recording this and the two new sources.

### 3.1 `measure()` must stop being NVML-only

This is the change most easily missed, and without it the feature reaches no
non-NVIDIA machine at all. `HardwareProbe::measure` today reads:

```rust
let Some(nvidia) = &self.nvidia else {
    return Ok(Vec::new());
};
Ok(nvidia.measure(&self.measurable))
```

An AMD-only Windows machine has no NVML handle, so this returns empty and every
per-tick figure the front end draws — including `shared_used` — never arrives.
`measure()` becomes a merge of two sources:

1. NVML samples for the devices NVML can read, exactly as now.
2. `adapter_memory()` readings, folded in by device index: `shared_used` always,
   and `vram_used` only where NVML did not already supply it (§1.5).

A device present in neither yields `GpuSample::unmeasured`, unchanged. The early
return on `!self.probed` stays: measuring before probing is still a programming
error with an empty answer.

`HardwareProbe` gains the state this needs — the device-index mapping resolved
once at probe time, and on Windows the PDH query handle held for the session.
Resolving the mapping per tick would mean re-enumerating DXGI adapters every two
seconds to answer a question whose answer cannot change within a session.

---

## 4. Per-platform sources

### 4.1 Windows

**Totals**, once at probe time: `CreateDXGIFactory1` → `IDXGIFactory1::EnumAdapters1`
→ `DXGI_ADAPTER_DESC1`, giving `DedicatedVideoMemory`, `SharedSystemMemory`,
`VendorId`, `DeviceId`, `Description` and `AdapterLuid`.

Adapters with the `DXGI_ADAPTER_FLAG_SOFTWARE` flag are skipped. This is the
same exclusion `is_real_hardware` already makes for the Microsoft Basic Render
Driver, made a second time because DXGI enumerates independently of `wgpu`.

**Usage**, per tick: PDH, opened once and re-collected each sample.

```
PdhOpenQueryW
PdhAddEnglishCounterW  \GPU Adapter Memory(*)\Dedicated Usage
PdhAddEnglishCounterW  \GPU Adapter Memory(*)\Shared Usage
PdhCollectQueryData
PdhGetFormattedCounterArrayW  PDH_FMT_LARGE
```

Both are raw gauges, not rates, so one collection yields a value — unlike a
rate counter, which needs two separated in time. The query handle is held for
the session rather than reopened per tick.

Instance names are `luid_0xHHHHHHHH_0xLLLLLLLL_phys_N`. The two hex groups are
the `LUID`'s `HighPart` and `LowPart`; `phys_N` distinguishes physical adapters
behind one LUID. Parsing is a pure function and is where the test coverage
concentrates. Instances that do not parse are skipped, not guessed at.

Matching PDH instances to DXGI adapters is by LUID. Matching DXGI adapters to
already-enumerated `GpuDevice`s is by PCI vendor and device ID, falling back to
the existing `normalise()` name comparison — the same function that already
dedupes NVML against `wgpu`, reused rather than reimplemented.

**`windows` crate features** required: `Win32_Graphics_Dxgi`,
`Win32_Graphics_Dxgi_Common`, `Win32_System_Performance`, `Win32_Foundation`.

**Unsafe code.** `osstat-llm` does not currently carry `#![deny(unsafe_code)]`
(`osstat-platform` does). Both DXGI and PDH are raw FFI. The module carries a
scoped `#[allow(unsafe_code, reason = "…")]` with the same discipline as
`osstat-platform/src/windows.rs`: every `unsafe` block gets a `// SAFETY:`
comment, and nothing borrowed from the OS outlives the call that produced it.

### 4.2 Linux

`/sys/class/drm/card*/device/`:

| File                  | Maps to        |
| --------------------- | -------------- |
| `mem_info_vram_total` | `vram_total`   |
| `mem_info_vram_used`  | `vram_used`    |
| `mem_info_gtt_total`  | `shared_total` |
| `mem_info_gtt_used`   | `shared_used`  |

GTT — the graphics translation table — is the aperture through which the GPU
addresses system memory, and is the Linux equivalent of the shared pool. These
files are amdgpu's. `i915`, `xe` and NVIDIA's proprietary driver expose no
adapter-wide equivalent, so on those the module returns nothing and the Overview
is unchanged from today. A missing file is an absent figure, never a zero.

Matching to a `GpuDevice` is by the PCI IDs in the sibling `vendor` and `device`
files, compared against `wgpu`'s `AdapterInfo::vendor` and `::device`. Name
matching is not used on Linux: DRM has no adapter name to compare.

Reading is plain `std::fs` on four small files per card, per tick. No `unsafe`,
no new dependency.

### 4.3 macOS

`adapter_memory()` returns an empty vector, and the Overview is unchanged.

This is not an omission. Apple Silicon has one memory pool, not two: the
existing `GpuSource::UnifiedMemory` path already reports the whole of system
memory as the GPU's, and the existing label already says "Unified memory (shared
with the system)". Adding `shared_total` there would draw a second meter
describing the same bytes as the first.

The one figure genuinely missing is live GPU memory usage, and the only route to
it is the undocumented `PerformanceStatistics` dictionary on the `IOAccelerator`
IOKit service. That was considered and declined: ADR-008's central rule is that
figures carry a source the user can weigh, and a private API that Apple can
change in a point release is a poor foundation for a number labelled "measured".
It also requires `objc2`/`core-foundation` dependencies for a single reading.

The file exists rather than being `cfg`'d away, matching `probe/nvml.rs`'s macOS
stub, so the merge logic in `adapter_memory.rs` compiles and runs identically on
all three platforms.

---

## 5. The LLM advisor must not see shared memory

`calculator.rs:295` builds the VRAM budget:

```rust
let best_known = devices.iter().filter_map(|device| device.vram_total).max();
```

`shared_total` **must not** be added to this, summed into it, or offered as a
fallback for it. Shared memory is system RAM reached across PCIe — roughly
30–80 GB/s against 400–1000 GB/s for on-card VRAM. A model that "fits" by
spilling into it does not fit in any sense the user means when they ask.

The calculator already models this correctly and already has a verdict for it:
system memory is weighed separately, and the outcome is `CpuOffload`, with an
estimate of layers moved and a throughput tier. Letting shared memory inflate
`vram_bytes` would convert an honest "fits with CPU offload" into a false "fits
in VRAM" — the exact failure ADR-008 calls the most damaging thing this feature
could do.

This gets a regression test naming the constraint, not just a comment.

### 5.1 A knock-on the reader should not be surprised by

Giving AMD and Intel adapters on Windows a real `vram_total` moves them off the
calculator's `GpuMemory::Unknown` fallback and onto real verdicts. That is a fix
— an unknown-VRAM machine currently gets the weakest advice osstat can give —
but it changes LLM advice on those machines, not only the Overview. It is stated
here so it is not discovered as a surprise.

---

## 6. Front end

`GpuContent` in `ui/src/pages/Overview.tsx` renders up to two `Meter`s. The
component needs no changes; it already takes a fraction, a label and a detail
string.

The four cases, in the order the code should read:

1. **Neither pool known** — today's message, unchanged: "Video memory is not
   reported by this source (Vulkan)."
2. **Dedicated only** — today's single meter, unchanged. Covers NVIDIA on Linux
   and any card whose shared pool is unreadable.
3. **Shared only** — one meter labelled "Shared memory", detail "borrowed from
   system memory". Covers an integrated GPU on Windows reporting
   `DedicatedVideoMemory: 0`. A dedicated meter reading `0 GB of 0 GB` is not
   drawn.
4. **Both** — two meters, "Dedicated" above "Shared".

`UnifiedMemory` keeps its own label and its single meter; it is case 2 and is
not restructured.

The "— estimated" suffix is decided per meter from that pool's own source:
`source` for dedicated, `shared_source` for shared. This is the front-end half
of §1.3, and without it the whole `shared_source` field is inert.

---

## 7. Testing

The probes need hardware; the logic does not. Coverage concentrates on pure
functions, which is what CI — with no GPU — can actually run.

**Pure, exhaustively tested:**

- LUID instance-name parsing: well-formed, wrong prefix, missing `phys`,
  non-hex, empty. Malformed input yields `None`, never a partial parse.
- The `AdapterMemory` → `GpuDevice`/`GpuSample` merge: every combination of
  present and absent pools, asserting §2.1's invariants and §1.4's source
  promotion.
- The amdgpu sysfs parser, against fixture files: all four present, `gtt_*`
  absent, a file containing trailing whitespace, a file containing garbage.
- PCI vendor/device matching, including the case where two adapters share a
  vendor.
- The `measure()` merge (§3.1): an NVML sample and an adapter reading for the
  same device combine without NVML's `vram_used` being overwritten; an adapter
  reading with no NVML sample still produces a sample; a device with neither
  yields `unmeasured`.
- Zero normalisation (§2.1): a source reporting `0` for either pool yields
  `None`, asserted per source rather than once at the merge.

**Guard tests, stating the rules this spec sets:**

- No source produces `shared_total` where the platform has no shared pool — the
  sibling of the existing `wgpu_never_claims_to_know_discrete_vram`.
- `shared_source` is `Some` exactly when `shared_total` is.
- **`shared_total` never reaches the LLM VRAM budget** (§5), asserted against a
  device carrying a large shared pool and no dedicated one.

**Front end**, in `App.test.tsx` alongside the existing "does not claim a VRAM
figure that wgpu cannot supply":

- Two meters render when both pools are present, one when only one is.
- No meter renders for a pool whose total is `0`.
- The "estimated" suffix follows each pool's own source: a device with
  `source: 'nvml'` and `shared_source: 'dxgi'` marks neither, while one with
  `source: 'wgpu'` marks only what `wgpu` supplied.

**Live probes**, which pass on a machine with no GPU because that is CI's
ordinary state: probing succeeds with an empty list, measuring before probing
yields nothing, and every sample indexes a device that exists. These mirror the
tests already in `probe.rs` rather than inventing a new shape.

**Not tested automatically**, and stated plainly rather than papered over: that
the Windows figures agree with Task Manager, and that PDH resolves on a
non-English Windows install (§1.2). Both need a real machine and belong in a
manual verification pass.

---

## 8. Open items

None blocking. Two things are deliberately deferred:

- **Per-process GPU memory.** The `GPU Process Memory` counter set exists and
  would make a process-table column. Different feature.
- **macOS live GPU usage.** Revisit only if Apple documents an API for it.
