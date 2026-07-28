# Interactive dashboard: live system metrics, process tree and selectable views

**Date:** 2026-07-28
**Status:** Implemented. Sections marked _(changed during implementation)_ record
where the built thing differs from the design, and why.
**Supersedes:** nothing. **Depends on:** ADR-002, ADR-003, ADR-007, ADR-008

## Context

M0 shipped a shell: one IPC command (`app_info`) and a static page listing what
does not exist yet. The deliberate plainness has done its job, and the next thing
the app needs is to actually show the machine it is running on.

This spec covers the _read_ side of M1 — system information, live metrics,
charts and the process tree — plus a selectable view layout, plus the ADR-008
hardware probe pulled forward from M4.

It deliberately excludes process termination.

## Goals

1. Every number on screen is real, measured on the user's machine.
2. The Overview and Processes pages are genuinely interactive: charts with
   history and hover, a tree that expands, sections that collapse.
3. The user chooses how the app is laid out, rather than the app choosing.
4. Nothing in the data path makes the idle-RAM or cold-start budgets worse in a
   way we cannot measure.

## Non-goals

- **Killing processes.** SIGTERM/SIGKILL escalation, critical-process guards and
  the ADR-006 elevation path get their own spec. A kill button without its guards
  is the single most damaging bug this app could ship, and it should not compete
  for review attention with chart colors.
- Ports, cleaner and LLM advisor pages. They remain honest empty states.
- The model registry and runnability arithmetic. Only the _hardware probe_ half
  of ADR-008 lands here.
- A light theme. `index.css` commits to dark deliberately; this is not the change
  that should revisit it.

---

## 1. Backend

### 1.1 Crate layout

Per ADR-003, capabilities are traits in `osstat-core`, implemented in
`osstat-platform`.

```
crates/osstat-core/       traits + domain types + the pure logic
crates/osstat-platform/   sysinfo-backed implementations
crates/osstat-llm/        NEW — the ADR-008 hardware probe only
src-tauri/                commands, sampler task, event emission
```

`osstat-llm` is where ADR-003 already places the hardware probe. It arrives
before its milestone; no ADR is amended.

### 1.2 A deliberate reading of ADR-003

ADR-003 asks for one implementation per OS. `sysinfo` is _already_ a
cross-platform abstraction, so three near-identical `windows.rs`/`linux.rs`/
`macos.rs` copies of the same `sysinfo` calls would be ceremony rather than
isolation, and would triple the surface that only CI can type-check.

**Decision:** the shared `sysinfo` adapter lives at the root of `osstat-platform`
as `sysinfo_source.rs`. The per-OS modules keep only what genuinely diverges —
disk-kind naming, user resolution, and later the critical-process lists. The
trait boundary that ADR-003 exists to protect is unchanged: call sites still
depend only on traits, and replacing `sysinfo` on one platform stays local.

### 1.3 Traits and types (`osstat-core`)

```rust
pub trait SystemInfoProvider {
    fn describe(&mut self) -> Result<SystemDescription>;  // OS, CPU model, disks, interfaces
    fn sample(&mut self) -> Result<MetricsSample>;        // per-tick scalars
}

pub trait ProcessProvider {
    fn processes(&mut self) -> Result<Vec<ProcessRecord>>;  // flat; arranged by the front-end
}

pub trait GpuProvider {
    fn devices(&mut self) -> Result<Vec<GpuDevice>>;  // enumeration, called once
    fn measure(&mut self) -> Result<Vec<GpuSample>>;  // per-tick, may be empty
}
```

The methods are named apart rather than all being `sample` _(changed during
implementation)_: one type implements both `SystemInfoProvider` and
`ProcessProvider` over a single `sysinfo::System`, and two methods of the same
name would force fully-qualified syntax at every call site. Every sampler takes
`&mut self` because a rate only exists relative to a previous reading.

`MetricsSample` is a small, flat, `Copy`-friendly struct — CPU total, per-core
vector, memory used/available/cached, swap, per-disk used/total, per-interface
rx/tx deltas, per-GPU utilisation/VRAM. It is what goes in the ring buffer and
what crosses IPC every tick, so it stays scalars only: no strings, no paths.

`ProcessRecord` is flat: `pid`, `parent_pid`, `name`, `exe`, `user`, `cpu`,
`memory`, `disk_read_rate`, `disk_write_rate`, `start_time`, `status`.

### 1.4 Pure logic in core (the part worth testing hardest)

**Tree building lives in the front-end, not here.** _(Changed during
implementation — this section originally placed it in `osstat-core`.)_

The reason is the diff. Because the sampler sends changes rather than
snapshots, whatever renders the tree has to maintain its own structure to apply
them to — so a tree assembled in Rust would be rebuilt on arrival and never
used. Keeping both would mean two implementations of the same delicate
orphan-and-cycle handling, drifting apart. It now lives once, in
`ui/src/lib/processTree.ts`, with the same cases covered:

- orphans (a parent PID not in the set) → promoted to roots rather than dropped
- cycles → broken deterministically, every process still present
- PID 0 and self-parenting roots, which differ per OS
- roll-ups, tested as an invariant: for any tree, the total equals the sum over
  every record, so collapsing a node can never hide load

What stays in `osstat-core` is identity ([`ProcessKey`]) and change detection
(`diff_processes`) — the parts the _sampler_ needs, tested with no OS involved.

**`MetricsHistory`** — a fixed-capacity ring buffer of `MetricsSample`, **600
slots**. The capacity is set by the longest history window at the _fastest_ tick
rate — 10 minutes at 1 s — so no combination of the two settings in 2.1 can ask
for more history than the buffer holds. At the 2 s default it covers 20 minutes;
at 5 s, 50. Pure, no I/O, tested for wraparound.

### 1.5 Sampling and transport

Per ADR-007: push from Rust on a tick, do not poll from the webview.

- One background task in `src-tauri` owns a single `sysinfo::System` and one
  `MetricsHistory`, and refreshes on the configured interval.
- **History lives in Rust, not React.** Charts fill instantly on first paint and
  survive a reload or a navigation change, instead of starting empty each time.
- Sampling **pauses when the window is minimised** — and deliberately _not_ when
  it merely loses focus _(clarified during implementation)_. Alt-tabbing away to
  make something happen and then coming back to look at the graph is the most
  common way this app will be used; a sampler that stopped on blur would erase
  exactly the history the user went to fetch. While paused the thread waits on a
  condition variable rather than ticking, so a minimised window costs nothing.

**Commands** (pull — initial paint and explicit refresh):

| Command               | Returns                                                                                       |
| --------------------- | --------------------------------------------------------------------------------------------- |
| `system_description`  | OS, kernel, hostname, uptime, CPU model, core counts, total memory, disk list, interface list |
| `metrics_history`     | recent samples, bounded by a `limit`, for the first chart paint                               |
| `process_list`        | every process, flat — the front-end's starting snapshot                                       |
| `gpu_devices`         | enumerated adapters, or `null` while the probe is still running (see 1.6)                     |
| `set_sample_interval` | changes the tick rate                                                                         |
| `set_sampling_paused` | suspends or resumes sampling                                                                  |

**Events** (push — per tick):

| Event            | Payload                                                                   |
| ---------------- | ------------------------------------------------------------------------- |
| `metrics:tick`   | one `MetricsSample`. Scalars only; small enough to ignore.                |
| `processes:tick` | **a diff**: `{ added: [], removed: [ProcessKey], changed: [] }`           |
| `gpus:ready`     | fired once, when the GPU probe finishes; the UI refetches the device list |

`removed` carries the full `ProcessKey` rather than a bare PID _(changed during
implementation)_: when a PID is recycled, the same number appears as removed and
as added in one tick, and only the start time tells them apart.

The process diff is not optional. Serialising 1000+ full process records every
2 s is precisely the cost ADR-007 pushes back on, and it would show up as webview
churn against the idle-RAM goal. Records are matched against the previous tick by
`(pid, start_time)` — PID alone is not an identity, because PIDs are reused.

**Comparison happens at display precision, not raw precision.** A raw CPU float
differs on nearly every process on every tick, which would put the entire table
in `changed` and defeat the diff entirely. A record counts as changed only if a
value moves at the precision the UI actually renders — CPU to 0.1 %, memory to
the nearest displayed unit. This is the difference between a diff that works and
a diff that is a full snapshot wearing a diff's name.

### 1.6 GPU probe (`osstat-llm`)

Full ADR-008 order, as chosen:

1. **NVIDIA** via `nvml-wrapper` — name, total/used/free VRAM, utilisation,
   temperature.
2. **Everything else** via `wgpu` adapter enumeration — name, backend, vendor,
   device type, and reported VRAM where the backend exposes it. No live
   utilisation; the UI must not imply otherwise.
3. **macOS** — unified memory reported _as_ unified memory, never as a
   VRAM number pretending to be discrete.
4. **No GPU at all** — returns an empty list, and the UI shows a real empty
   state. CI runners have no GPU, so this is the continuously-exercised path,
   exactly as ADR-008 predicted.

**Fields differ by source, and the UI must say which source it used.** A `wgpu`
VRAM figure is a heuristic; an NVML figure is a measurement. Presenting a
heuristic as a measurement is the failure mode ADR-008 explicitly warns about,
so `GpuDevice` carries a `source: Nvml | Wgpu | UnifiedMemory` discriminant and
the UI labels estimates as estimates.

**Bundle-size obligation.** ADR-008 asks for `wgpu`'s installer footprint to be
measured when it lands. It lands here, so this spec inherits that: record the
release-binary delta from adding `wgpu` + `nvml-wrapper`, against the ~20 MB
budget. If it is material, note it for a lighter platform-specific probe.

---

## 2. Front end

### 2.1 Shell and the two view settings

Two **independent** preferences, not three bundled presets:

| Setting          | Values                                     | Default  |
| ---------------- | ------------------------------------------ | -------- |
| Navigation       | Sidebar · Top tabs · Icon rail             | Sidebar  |
| Page layout      | One page (collapsible sections) · Sub-tabs | One page |
| Refresh interval | 1 s · 2 s · 5 s · Paused                   | 2 s      |
| History window   | 1 min · 5 min · 10 min                     | 5 min    |

All three navigation components consume one `NAV_ITEMS` model, so adding a
capability is one array entry rather than three edits. `SectionContainer` takes
the same section components and either stacks them collapsibly or presents them
as sub-tabs — the sections themselves know nothing about which mode is active.

This orthogonality is why it is cheap: six combinations, roughly the code of
three, and no arbitrary pairing of "icon rail" with "sub-tabs".

### 2.2 Preferences

Persisted to `localStorage` behind a `preferences` module whose interface is the
one a Rust-backed config store would expose later (`get`/`set`/`subscribe`,
async-tolerant). A real config file is the right end state and arrives when the
cleaner needs actual settings; it is not worth pulling in for four enums now.

Unknown or corrupt stored values fall back to defaults rather than throwing —
a preferences read must never be able to white-screen the app.

### 2.3 Routing

A `Route` union plus context. No `react-router`: six pages in a desktop app with
no URL bar and no back button does not justify the dependency, and the router's
main value — URLs — is not available to us.

### 2.4 Page and section structure

```
Overview   → Cpu · Memory · Disks · Network · Gpu
Processes  → the tree (single section)
Ports · Cleaner · LlmAdvisor → empty states naming the milestone
Settings   → the table in 2.1
```

Each Overview section is an independent component owning its own data
subscription, so sub-tab mode can mount only the visible one.

---

## 3. Data visualisation

Charts are ECharts, imported modularly from `echarts/core` — `LineChart`,
`BarChart`, `HeatmapChart` plus only the components used, with `CanvasRenderer`.
The barrel import is ~330 KB gzipped; this should land nearer 150 KB. Record the
actual delta against the installer budget alongside the `wgpu` measurement.

A ~60-line `useChart` hook rather than `echarts-for-react`: it owns init,
`setOption`, `ResizeObserver` and disposal. **Chart options are produced by pure
builder functions**, so they are unit-testable without rendering a canvas — the
tests assert on the option object, not on pixels.

### 3.1 Form per dataset

Chosen by the data's job, before any color was picked:

| Data                                  | Job                             | Form                                                             |
| ------------------------------------- | ------------------------------- | ---------------------------------------------------------------- |
| CPU total over time                   | trend, one series               | area line, single hue, **no legend** (the title names it)        |
| Per-core utilisation                  | compare magnitude across a grid | **heatmap**, sequential ramp — _not_ 16 categorical hues         |
| Memory over time                      | part-to-whole over time         | stacked area, 2 series                                           |
| Network throughput                    | two distinct series, same unit  | 2-line, shared axis                                              |
| Disk capacity                         | a ratio against a limit         | **meter** (same-ramp track) — not a pie, not a one-bar bar chart |
| Per-process CPU in the tree           | ratio against a limit, per row  | inline meter                                                     |
| GPU VRAM                              | ratio against a limit           | meter                                                            |
| GPU utilisation over time             | trend, one series               | area line                                                        |
| Headline numbers (CPU %, RAM, uptime) | single current values           | **stat tiles**, not charts                                       |

Two rules this table encodes, both easy to get wrong:

- **16 cores is not 16 series.** Per-core utilisation is magnitude, so it gets one
  hue and a sequential ramp. Generating 16 distinguishable hues is impossible and
  the attempt breaks every accessibility check.
- **Network down/up share one axis.** Both are bytes/second, so they belong on one
  scale. A second y-axis is never correct here.

### 3.2 Palette — validated, not eyeballed

osstat's chart surface is the raised panel, `--color-surface-raised`
`oklch(0.24 0.024 265)` = **`#1a1f2b`**. All checks were run against that surface
in dark mode with `scripts/validate_palette.js`.

**Series colors** (dark steps of the reference categorical palette):

| Slot | Hue    | Hex       |
| ---- | ------ | --------- |
| 1    | blue   | `#3987e5` |
| 2    | orange | `#d95926` |
| 3    | aqua   | `#199e70` |

Validator results on `#1a1f2b`:

- All eight reference slots pass every adjacent-pair gate (worst adjacent CVD
  ΔE 8.4, worst adjacent normal-vision ΔE 19.3, all ≥ 3:1 contrast).
- **The first three slots pass under `--pairs all`** — worst all-pairs CVD ΔE 9.4,
  normal-vision ΔE 20.9. No osstat chart needs more than three series, so three
  is the cap and no chart may exceed it.

**A finding that changes existing code:** osstat's current
`--color-accent` `oklch(0.72 0.15 232)` = `#00b3f2` **fails the lightness band**
(L 0.721; the band is 0.48–0.67). It must **not** be used as a chart series
color. It remains the UI accent — selection, focus rings, active nav — where the
band does not apply. Charts use slot 1 `#3987e5` as their primary hue instead.

**Sequential ramp** (per-core heatmap, meters): the blue ramp. _(Corrected
during implementation.)_ It is used **dark→bright**, not light→dark. A
sequential ramp's low end should recede toward the surface it is drawn on, and
this surface is dark; run the other way, an idle CPU core glared almost white
while a saturated one sank into the background — the encoding inverted, with the
brightest thing on screen being the part doing nothing. Same six validated
steps, reversed, as `SEQUENTIAL_ON_DARK`.

**Status colors** (`good #0ca30c`, `warning #fab219`, `serious #ec835a`,
`critical #d03b3b`) are reserved for state and never reused as a series. They
apply to one thing here: a disk crossing a fullness threshold. Per the rule they
ship with **an icon and a label** ("Low space"), never color alone — so a disk
meter is a single hue plus an explicit status badge, not a bar that silently
turns red.

**Text wears text tokens**, never the series color. Values and labels stay in the
existing ink colors; a colored mark beside them carries identity.

### 3.3 Interaction

- Line and area charts get a **crosshair plus tooltip** by default, reading all
  series at the hovered timestamp.
- Meters and heatmap cells get a per-mark hover tooltip (core number, exact
  percentage).
- With two series a legend is always present, and both are direct-labeled, so
  identity is never carried by color alone.
- Charts render only what the History window setting covers.

---

## 4. Process tree

- PPID hierarchy, expandable, with cumulative CPU/memory/IO on collapsed parents.
- Search by name, PID or user. Matching inside a collapsed subtree auto-expands
  the path to the match — a filter that silently hides matches is worse than none.
- Sortable columns. Sorting in tree mode sorts siblings within their parent;
  a flat-list toggle sorts globally.
- **Virtualized**: hand-rolled windowing over the flattened visible-row array. One
  list does not justify `react-window`, and the flattening step is needed anyway
  for expand/collapse.
- Read-only. No kill affordance, not even a disabled one — a disabled kill button
  invites a bug report and promises something this phase does not deliver.

---

## 5. Testing and gates

| Area              | Coverage                                                                                                                                                                                                             |
| ----------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `osstat-core`     | Tree building: orphans, cycles, self-parenting, PID reuse. Roll-up invariant: root total equals the sum over all records, for any generated tree. Ring buffer wraparound.                                            |
| `osstat-platform` | Smoke tests per platform on the CI matrix: a sample yields plausible, non-empty data.                                                                                                                                |
| `osstat-llm`      | The **no-GPU path** is the one CI exercises and must degrade to an empty list, never an error. `source` discriminant is set correctly per branch.                                                                    |
| `src-tauri`       | Diff correctness: a synthetic two-tick sequence produces exactly the expected added/removed/changed sets, including the PID-reuse case.                                                                              |
| UI (vitest)       | Preference persistence and corrupt-value fallback; all three navigation modes render the same route set; section collapse; tree expand/collapse and search auto-expand; **chart option builders** as pure functions. |
| Benchmark         | A 500-process refresh **under 50 ms** — M1's committed gate, split across both languages because the work is (see below).                                                                                            |

### 5.1 Measured results

The M1 gate covers the whole refresh, and the refresh is now half Rust and half
TypeScript, so both are benchmarked — `just bench` runs them together. Measuring
only the Rust half would report a number nobody experiences.

| Step                                                     | Measured     | Where                                               |
| -------------------------------------------------------- | ------------ | --------------------------------------------------- |
| `sysinfo` read of the real process table (474 processes) | **23.8 ms**  | `crates/osstat-platform/benches/process_refresh.rs` |
| `diff_processes` over 500 records                        | **0.056 ms** | same                                                |
| `buildTree` + `flattenTree`, 500 processes               | **0.23 ms**  | `ui/src/lib/processTree.bench.ts`                   |
| **Total tick**                                           | **≈ 24 ms**  | against a 50 ms gate                                |

The `sysinfo` read dominates by two orders of magnitude, which is worth knowing:
osstat's own logic is not where this budget will be lost.

Bundle and binary measurements, owed to ADR-008 and the ~20 MB installer goal:

| Measurement                             | Result                         |
| --------------------------------------- | ------------------------------ |
| Front-end bundle, modular ECharts       | 754 KB raw, **251 KB gzipped** |
| Release binary, `wgpu` + NVML linked in | **4.57 MB**                    |

Against a ~20 MB installer goal, both are comfortable. ADR-008 worried that
`wgpu` would be "a heavy dependency for what is ultimately adapter enumeration";
at 4.57 MB total for the whole binary, that worry does not materialise on
Windows. The `wgpu`-only delta was **not isolated** — that needs a second
release build with the probe removed — and the figure should be re-taken on
Linux and macOS before v0.1.0, since the backends differ.

### 5.2 A security setting ECharts forced off

`app.security.freezePrototype` was `true`. It is now **`false`**, and that is a
real reduction in hardening, not a tidy-up.

ECharts (through zrender) assigns to `constructor` while setting up its class
hierarchy. With `Object.prototype` frozen, that assignment throws, the whole
bundle dies before React mounts, and the window renders blank white with no
message. It is not a configuration that can be worked around from this side:
the freeze is applied to the webview before any application script runs, and
there is no way to exempt one library.

So the options were ECharts or the prototype freeze, and this spec chose
ECharts. What is lost is one layer of defence against prototype-pollution, on a
webview that loads no third-party script, makes no network requests, and renders
no remote content — which is what makes the trade acceptable rather than merely
necessary. If that changes, this decision has to be revisited.

Two related changes came out of the same investigation:

- **`devCsp`** was added so the strict production CSP survives. Vite's React
  Refresh preamble is an inline `<script>`, which `script-src 'self'` blocks —
  meaning `tauri dev` had _never_ rendered this app. Production keeps its
  `script-src 'self'`; only development relaxes.
- **A startup error is now displayed instead of a blank page.** A webview that
  throws before React mounts paints nothing, which tells a user nothing and
  gives them nothing to paste into an issue. `ui/src/main.tsx` now writes the
  error where the UI would have been. This is how the ECharts failure was
  eventually diagnosed.

Measurements to record (both are pre-existing obligations coming due):

- Release-binary delta from `wgpu` + `nvml-wrapper`, against the ~20 MB budget
  (ADR-008 asks for this explicitly).
- Bundle delta from the modular ECharts import.

## 6. Risks

- **Scope.** This is most of M1's read side plus half of ADR-008. The plan must
  sequence the GPU probe as a separable chunk that can be dropped without
  stranding the rest.
- **`wgpu` weight.** It may be material against the installer budget. The
  measurement above is the trigger for reconsidering, not a formality.
- **The 50 ms benchmark constrains the diff design**, not just the sampler. If
  diffing itself is expensive at 1000 processes, the tick rate — not the
  benchmark — is what gives.
- **PID reuse** is the subtle correctness risk in the diff. It is called out in
  both the core tests and the `src-tauri` tests for that reason.

## 7. Decisions deferred

- A Rust-backed config store, when the cleaner needs real settings.
- Light theme.
- Sensors/temperature charts (`sysinfo` exposes components; post-v1 per roadmap).
- Per-disk I/O rates — `sysinfo` gives per-_process_ IO, not per-disk; a per-disk
  chart needs a platform-specific source and is not worth it here.
