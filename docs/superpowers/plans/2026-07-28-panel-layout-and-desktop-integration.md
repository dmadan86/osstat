# Panel Layout and Desktop Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the user order, resize and hide the Overview panels with the arrangement persisted, and make osstat a resident desktop app that starts at sign-in, lives in the notification area and costs almost nothing while hidden.

**Architecture:** Phase A is entirely front-end: a `PanelLayout[]` stored beside the existing view preferences, reconciled against the code's section list on every render so the two can never disagree, rendered by a 12-column flow grid. Phase B is mostly Rust: the sampler's `paused: bool` becomes a three-state `Activity`, a tray icon is created in `setup`, and two official Tauri plugins provide single-instance and autostart.

**Tech Stack:** React 19 + TypeScript (`exactOptionalPropertyTypes`, `noUncheckedIndexedAccess`), Tailwind v4, ECharts 6, Vitest; Rust 2024 + Tauri 2.11, `tauri-plugin-autostart`, `tauri-plugin-single-instance`.

**Spec:** [`docs/superpowers/specs/2026-07-28-panel-layout-and-desktop-integration-design.md`](../specs/2026-07-28-panel-layout-and-desktop-integration-design.md)

## Global Constraints

- **Clippy pedantic with `-D warnings`.** No `unwrap`, `expect` or `panic` outside `#[cfg(test)]` blocks. `missing_docs = "warn"` — every public item, module and bench file needs a doc comment.
- **`cargo` is not on a bare shell's PATH in some setups.** If a cargo command reports "program not found", prefix with `$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"` (PowerShell).
- **TypeScript `exactOptionalPropertyTypes` is on.** Never assign `undefined` to an optional property; use a conditional spread — `...(x > 0 ? { max: x } : {})`.
- **TypeScript `noUncheckedIndexedAccess` is on.** `array[i]` is `T | undefined`; narrow before use.
- **ESLint `react-hooks/refs`:** never write a ref during render. Ref writes belong in effects or event handlers.
- **Every IPC call goes through a named function in `ui/src/lib/ipc.ts`.** No raw `invoke` at a call site. Command name strings live in `COMMANDS`, event names in `EVENTS`.
- **Any Rust type crossing IPC** needs `#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]` and `ts(export)`, then `just bindings` to regenerate `ui/src/bindings/`.
- **Commit message format is conventional-commits**, enforced by commitlint. Body lines wrap at 72 characters. A blank line before the `Co-Authored-By` trailer.
- **Run `just ci` before declaring a phase done.** Note: it currently fails on `prettier --check` over untracked `.claude-flow/*.json`, which is unrelated to this work — see the end of this plan.

## Phase boundaries

- **Phase A (Tasks 1–7)** — Overview arrangement. Ships on its own.
- **Phase B (Tasks 8–12)** — Tray, startup, background sampling. Ships on its own.

---

## Task 1: Time axes plot real time

The x-axis is `type: 'category'` with one evenly-spaced slot per sample, so it draws a 5-second gap exactly as wide as a 2-second one. Phase B makes irregular spacing routine; this fixes it first, in isolation.

Switching to `type: 'time'` changes the tooltip header, which ECharts would otherwise render as a full date. So the tooltip's formatter moves into `base()`, which now takes the value formatter its charts need.

**Files:**

- Modify: `ui/src/charts/options.ts:35-59` (`base`, `timeAxis`), `:71-179` (the three chart builders)
- Test: `ui/src/charts/options.test.ts`

**Interfaces:**

- Consumes: nothing from earlier tasks.
- Produces: `base(formatValue: (value: number) => string): EChartsOption` — internal. `timeAxis(): NonNullable<EChartsOption['xAxis']>` — internal, no longer takes points. The three exported builders keep their existing signatures: `percentAreaOption(points: readonly TimePoint[], name: string)`, `memoryAreaOption(used: readonly TimePoint[], total: number)`, `throughputOption(down: readonly TimePoint[], up: readonly TimePoint[])`.

- [ ] **Step 1: Write the failing tests**

Append to `ui/src/charts/options.test.ts`:

```ts
describe('time axes', () => {
  const points: TimePoint[] = [
    { at: 1_700_000_000_000, value: 10 },
    { at: 1_700_000_002_000, value: 20 },
    // A five-second gap: this is what a stretch sampled in the background looks
    // like, and a category axis would draw it the same width as the two-second
    // step above it.
    { at: 1_700_000_007_000, value: 30 },
  ];

  it('plots against real time rather than evenly spaced slots', () => {
    const option = percentAreaOption(points, 'CPU');
    expect(option.xAxis).toMatchObject({ type: 'time' });
  });

  it('carries the timestamp in every datum, so spacing survives into the chart', () => {
    const option = percentAreaOption(points, 'CPU');
    const series = Array.isArray(option.series) ? option.series[0] : undefined;
    expect(series?.data).toEqual([
      [1_700_000_000_000, 10],
      [1_700_000_002_000, 20],
      [1_700_000_007_000, 30],
    ]);
  });

  it('uses real time on the memory and throughput charts too', () => {
    expect(memoryAreaOption(points, 1_000).xAxis).toMatchObject({ type: 'time' });
    expect(throughputOption(points, points).xAxis).toMatchObject({ type: 'time' });
  });

  it('heads the tooltip with a clock time, not an ISO timestamp', () => {
    const option = percentAreaOption(points, 'CPU');
    const formatter = (option.tooltip as { formatter?: unknown }).formatter;
    expect(typeof formatter).toBe('function');

    const rendered = (formatter as (params: unknown) => string)([
      {
        axisValue: 1_700_000_000_000,
        seriesName: 'CPU',
        value: [1_700_000_000_000, 42],
        marker: '<i></i>',
      },
    ]);

    expect(rendered).toContain('42%');
    expect(rendered).not.toContain('1700000000000');
  });

  it('formats each chart in its own unit', () => {
    const bytes = memoryAreaOption(points, 1_000);
    const formatter = (bytes.tooltip as { formatter: (params: unknown) => string }).formatter;
    const rendered = formatter([
      {
        axisValue: 1_700_000_000_000,
        seriesName: 'Used',
        value: [1_700_000_000_000, 1_048_576],
        marker: '<i></i>',
      },
    ]);
    expect(rendered).toContain('MB');
  });
});
```

Make sure `TimePoint`, `memoryAreaOption` and `throughputOption` are in the file's import list at the top.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npm run test --workspace @osstat/ui -- options`
Expected: FAIL — `xAxis` is `{ type: 'category' }`, and `tooltip.formatter` is `undefined`.

- [ ] **Step 3: Rewrite `base` and `timeAxis`**

Replace `ui/src/charts/options.ts:34-59` with:

```ts
/** One row of an axis-triggered tooltip, as ECharts hands it to a formatter. */
interface TooltipRow {
  /** The x value under the pointer. On a time axis, epoch milliseconds. */
  axisValue: number;
  /** The series this row describes. */
  seriesName: string;
  /** The datum, as the `[time, value]` pair the series was given. */
  value: [number, number];
  /** The coloured swatch ECharts pre-renders for this series. */
  marker: string;
}

/**
 * Shared axis, grid and tooltip chrome.
 *
 * The tooltip is formatted here rather than per series because a `time` axis
 * heads each tooltip with the axis value, and ECharts' default rendering of
 * that is a full ISO-style timestamp. Passing the value formatter in keeps one
 * tooltip shape across the charts while each still reads in its own unit.
 *
 * @param formatValue Renders a y value in the chart's unit.
 */
function base(formatValue: (value: number) => string): EChartsOption {
  return {
    animation: false,
    grid: { left: 48, right: 12, top: 12, bottom: 22, containLabel: false },
    tooltip: {
      trigger: 'axis',
      backgroundColor: INK.surface,
      borderColor: INK.axis,
      textStyle: { color: INK.primary, fontSize: 11 },
      axisPointer: { type: 'line', lineStyle: { color: INK.muted, width: 1 } },
      formatter: (params: unknown) => {
        const rows = (Array.isArray(params) ? params : [params]) as TooltipRow[];
        const first = rows[0];
        if (first === undefined) return '';

        const body = rows
          .map((row) => `${row.marker}${row.seriesName} ${formatValue(Number(row.value[1]))}`)
          .join('<br/>');

        return `${formatClock(Number(first.axisValue))}<br/>${body}`;
      },
    },
  };
}

/**
 * An axis of real time, labelled with local clock times.
 *
 * `type: 'time'` rather than `'category'`: samples are not evenly spaced. The
 * sampler slows to 5 s while the window is hidden and stops entirely while it
 * is paused, and a category axis would draw both of those as though no time had
 * passed.
 */
function timeAxis(): NonNullable<EChartsOption['xAxis']> {
  return {
    type: 'time',
    axisLine: { lineStyle: { color: INK.axis } },
    axisTick: { show: false },
    axisLabel: {
      color: INK.muted,
      fontSize: 10,
      hideOverlap: true,
      formatter: (value: number) => formatClock(value),
    },
  };
}
```

- [ ] **Step 4: Update the three chart builders**

In `percentAreaOption`: replace `...base(),` with `...base(formatPercent),`, replace `xAxis: timeAxis(points),` with `xAxis: timeAxis(),`, replace the series `data:` line with `data: points.map((point) => [point.at, point.value]),`, and delete the series-level `tooltip: { valueFormatter: ... }` line.

In `memoryAreaOption`: `...base(formatBytes),`, `xAxis: timeAxis(),`, `data: used.map((point) => [point.at, point.value]),`, delete the series-level `tooltip:` line.

In `throughputOption`: `...base(formatRate),`, `xAxis: timeAxis(),`, `data: down.map((point) => [point.at, point.value]),` and `data: up.map((point) => [point.at, point.value]),`, delete both series-level `tooltip:` lines.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `npm run test --workspace @osstat/ui -- options`
Expected: PASS, including the pre-existing tests in that file.

- [ ] **Step 6: Typecheck and lint**

Run: `npm run typecheck --workspace @osstat/ui; npm run lint --workspace @osstat/ui`
Expected: both exit 0.

- [ ] **Step 7: Commit**

```bash
git add ui/src/charts/options.ts ui/src/charts/options.test.ts
git commit -m "fix(charts): plot samples against real time

The x-axis was type: 'category', one evenly spaced slot per sample, so
the gap a minimised window leaves in history was drawn as though no time
had passed. Background sampling at a slower rate is about to make that
routine rather than rare.

The tooltip formatter moves into base() because a time axis heads each
tooltip with the axis value, and ECharts renders that as a full
timestamp by default. Each chart passes the formatter for its own unit.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 2: The panel layout model

Pure functions only — no React. This is where the majority of the tests live, because reconciliation is what stops a saved layout and a shipped release from disagreeing about which panels exist.

**Files:**

- Create: `ui/src/lib/panelLayout.ts`
- Test: `ui/src/lib/panelLayout.test.ts`

**Interfaces:**

- Consumes: nothing.
- Produces:
  - `type PanelSpan = 4 | 6 | 8 | 12`
  - `type PanelHeight = 'short' | 'normal' | 'tall'`
  - `interface PanelLayout { id: string; span: PanelSpan; height: PanelHeight; hidden: boolean }`
  - `const PANEL_SPANS: readonly PanelSpan[]`
  - `const PANEL_HEIGHT_NAMES: readonly PanelHeight[]`
  - `const PANEL_HEIGHTS: Record<PanelHeight, number>`
  - `const SPAN_LABELS: Record<PanelSpan, string>`
  - `const HEIGHT_LABELS: Record<PanelHeight, string>`
  - `const NARROW_WIDTH = 900`
  - `defaultPanelLayout(id: string): PanelLayout`
  - `coercePanelLayout(raw: unknown): PanelLayout[]`
  - `reconcileLayout(stored: readonly PanelLayout[], ids: readonly string[]): PanelLayout[]`
  - `reorder<T>(list: readonly T[], from: number, to: number): T[]`
  - `moveById(list: readonly PanelLayout[], id: string, delta: number): PanelLayout[]`
  - `updatePanel(list: readonly PanelLayout[], id: string, change: Partial<Omit<PanelLayout, 'id'>>): PanelLayout[]`

- [ ] **Step 1: Write the failing tests**

Create `ui/src/lib/panelLayout.test.ts`:

```ts
import { describe, expect, it } from 'vitest';

import {
  coercePanelLayout,
  defaultPanelLayout,
  moveById,
  reconcileLayout,
  reorder,
  updatePanel,
  type PanelLayout,
} from './panelLayout';

const IDS = ['cpu', 'memory', 'disks', 'network', 'gpu'];

function stored(...entries: Array<Partial<PanelLayout> & { id: string }>): PanelLayout[] {
  return entries.map((entry) => ({ ...defaultPanelLayout(entry.id), ...entry }));
}

describe('coercePanelLayout', () => {
  it('treats anything that is not an array as no stored layout', () => {
    expect(coercePanelLayout(undefined)).toEqual([]);
    expect(coercePanelLayout(null)).toEqual([]);
    expect(coercePanelLayout('cpu,memory')).toEqual([]);
    expect(coercePanelLayout({ cpu: 12 })).toEqual([]);
  });

  it('drops entries with no usable id', () => {
    expect(coercePanelLayout([{ span: 6 }, { id: '', span: 6 }, { id: 'cpu' }])).toEqual([
      defaultPanelLayout('cpu'),
    ]);
  });

  it('replaces an illegal span or height with the default rather than the whole entry', () => {
    expect(coercePanelLayout([{ id: 'cpu', span: 5, height: 'enormous', hidden: 'yes' }])).toEqual([
      { id: 'cpu', span: 12, height: 'normal', hidden: false },
    ]);
  });

  it('keeps legal values untouched', () => {
    const entry = { id: 'gpu', span: 4, height: 'tall', hidden: true };
    expect(coercePanelLayout([entry])).toEqual([entry]);
  });
});

describe('reconcileLayout', () => {
  it('keeps the stored order', () => {
    const result = reconcileLayout(stored({ id: 'gpu' }, { id: 'cpu' }), ['cpu', 'gpu']);
    expect(result.map((panel) => panel.id)).toEqual(['gpu', 'cpu']);
  });

  it('appends a section the stored layout has never heard of', () => {
    // The case that matters: a release adds a panel, and someone with a saved
    // layout must still see it.
    const result = reconcileLayout(stored({ id: 'gpu' }, { id: 'cpu' }), IDS);
    expect(result.map((panel) => panel.id)).toEqual(['gpu', 'cpu', 'memory', 'disks', 'network']);
  });

  it('gives an appended section the default size, not the previous one', () => {
    const result = reconcileLayout(stored({ id: 'cpu', span: 4 }), ['cpu', 'memory']);
    expect(result[1]).toEqual(defaultPanelLayout('memory'));
  });

  it('drops a stored id that no longer names a section', () => {
    const result = reconcileLayout(stored({ id: 'ports' }, { id: 'cpu' }), ['cpu']);
    expect(result.map((panel) => panel.id)).toEqual(['cpu']);
  });

  it('keeps only the first of a duplicated id, so a panel cannot render twice', () => {
    const result = reconcileLayout(stored({ id: 'cpu', span: 4 }, { id: 'cpu', span: 8 }), ['cpu']);
    expect(result).toHaveLength(1);
    expect(result[0]?.span).toBe(4);
  });

  it('returns the canonical order when nothing is stored', () => {
    expect(reconcileLayout([], IDS).map((panel) => panel.id)).toEqual(IDS);
  });

  it('repairs an illegal span that reached it unchecked', () => {
    const rogue = [
      { id: 'cpu', span: 7, height: 'normal', hidden: false },
    ] as unknown as PanelLayout[];
    expect(reconcileLayout(rogue, ['cpu'])[0]?.span).toBe(12);
  });
});

describe('reorder', () => {
  it('moves an item forwards', () => {
    expect(reorder(['a', 'b', 'c', 'd'], 0, 2)).toEqual(['b', 'c', 'a', 'd']);
  });

  it('moves an item backwards', () => {
    expect(reorder(['a', 'b', 'c', 'd'], 3, 1)).toEqual(['a', 'd', 'b', 'c']);
  });

  it('is a no-op when the destination is where the item already is', () => {
    expect(reorder(['a', 'b', 'c'], 1, 1)).toEqual(['a', 'b', 'c']);
  });

  it('leaves the list alone rather than corrupting it on an out-of-range index', () => {
    expect(reorder(['a', 'b'], -1, 1)).toEqual(['a', 'b']);
    expect(reorder(['a', 'b'], 0, 9)).toEqual(['a', 'b']);
  });

  it('does not mutate its input', () => {
    const original = ['a', 'b', 'c'];
    reorder(original, 0, 2);
    expect(original).toEqual(['a', 'b', 'c']);
  });
});

describe('moveById', () => {
  it('moves a panel up', () => {
    const result = moveById(stored({ id: 'a' }, { id: 'b' }, { id: 'c' }), 'c', -1);
    expect(result.map((panel) => panel.id)).toEqual(['a', 'c', 'b']);
  });

  it('refuses to move the first panel up, rather than wrapping it to the end', () => {
    const list = stored({ id: 'a' }, { id: 'b' });
    expect(moveById(list, 'a', -1)).toEqual(list);
  });

  it('refuses to move the last panel down', () => {
    const list = stored({ id: 'a' }, { id: 'b' });
    expect(moveById(list, 'b', 1)).toEqual(list);
  });

  it('ignores an id that is not in the list', () => {
    const list = stored({ id: 'a' });
    expect(moveById(list, 'nope', 1)).toEqual(list);
  });
});

describe('updatePanel', () => {
  it('changes one panel and leaves the others alone', () => {
    const result = updatePanel(stored({ id: 'a' }, { id: 'b' }), 'b', { span: 6, hidden: true });
    expect(result[0]).toEqual(defaultPanelLayout('a'));
    expect(result[1]).toEqual({ id: 'b', span: 6, height: 'normal', hidden: true });
  });

  it('ignores an unknown id', () => {
    const list = stored({ id: 'a' });
    expect(updatePanel(list, 'b', { span: 4 })).toEqual(list);
  });
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npm run test --workspace @osstat/ui -- panelLayout`
Expected: FAIL — cannot resolve `./panelLayout`.

- [ ] **Step 3: Write the implementation**

Create `ui/src/lib/panelLayout.ts`:

```ts
/**
 * How the Overview panels are arranged: their order, their size, and whether
 * they are shown at all.
 *
 * Everything here is a pure function over plain data, deliberately: the rules
 * that matter — what happens when a saved layout predates the release reading
 * it — are the ones hardest to check by clicking around, and easiest to check
 * in a test.
 *
 * Order is the array's order. There is no index field, because two
 * representations of the same fact can disagree and one cannot.
 */

/** Twelfths of a row a panel occupies. */
export type PanelSpan = 4 | 6 | 8 | 12;

/** How much vertical room a panel's body gets. */
export type PanelHeight = 'short' | 'normal' | 'tall';

/** One panel's placement. */
export interface PanelLayout {
  /** Matches the `SectionSpec.id` of the panel this describes. */
  id: string;
  /** Width, in twelfths of a row. */
  span: PanelSpan;
  /** Named height, resolved to pixels by {@link PANEL_HEIGHTS}. */
  height: PanelHeight;
  /** Whether the user has hidden this panel. */
  hidden: boolean;
}

/** Every width a panel may take, narrowest first. */
export const PANEL_SPANS: readonly PanelSpan[] = [4, 6, 8, 12];

/** Every height a panel may take, shortest first. */
export const PANEL_HEIGHT_NAMES: readonly PanelHeight[] = ['short', 'normal', 'tall'];

/** Pixels of chart height each named height is worth. */
export const PANEL_HEIGHTS: Record<PanelHeight, number> = {
  short: 96,
  normal: 140,
  tall: 220,
};

/** What each width is called in the UI. */
export const SPAN_LABELS: Record<PanelSpan, string> = {
  4: 'Third',
  6: 'Half',
  8: 'Two-thirds',
  12: 'Full',
};

/** What each height is called in the UI. */
export const HEIGHT_LABELS: Record<PanelHeight, string> = {
  short: 'Short',
  normal: 'Normal',
  tall: 'Tall',
};

/**
 * Content width below which every panel spans the full row.
 *
 * A third-width chart at this size is unreadable, so honouring the stored span
 * would serve nobody. The stored value is left alone and applies again as soon
 * as there is room for it.
 */
export const NARROW_WIDTH = 900;

/** How a panel is placed before the user has said otherwise. */
export function defaultPanelLayout(id: string): PanelLayout {
  return { id, span: 12, height: 'normal', hidden: false };
}

/** Whether `value` is a width a panel may take. */
function isSpan(value: unknown): value is PanelSpan {
  return PANEL_SPANS.some((span) => span === value);
}

/** Whether `value` is a height a panel may take. */
function isHeight(value: unknown): value is PanelHeight {
  return PANEL_HEIGHT_NAMES.some((height) => height === value);
}

/**
 * Repairs one entry, field by field.
 *
 * Each field falls back on its own so that one unrecognised value — a size
 * removed in a later version, a hand-edited record — costs only that field
 * rather than discarding a layout the user built.
 */
function normalise(entry: PanelLayout): PanelLayout {
  return {
    id: entry.id,
    span: isSpan(entry.span) ? entry.span : 12,
    height: isHeight(entry.height) ? entry.height : 'normal',
    hidden: entry.hidden === true,
  };
}

/**
 * Turns arbitrary stored data into a usable layout.
 *
 * @param raw Anything that came out of storage.
 * @returns Valid entries; an empty array if there is nothing usable, which
 *   reconciliation then fills from the canonical section list.
 */
export function coercePanelLayout(raw: unknown): PanelLayout[] {
  if (!Array.isArray(raw)) return [];

  const panels: PanelLayout[] = [];

  for (const candidate of raw) {
    if (typeof candidate !== 'object' || candidate === null) continue;

    const entry = candidate as Partial<Record<keyof PanelLayout, unknown>>;
    if (typeof entry.id !== 'string' || entry.id === '') continue;

    panels.push(normalise({ ...defaultPanelLayout(entry.id), ...entry } as PanelLayout));
  }

  return panels;
}

/**
 * Reconciles a stored layout against the sections the code actually has.
 *
 * The stored list is never treated as the authority on what exists — it is a
 * record of preferences about things whose existence is decided elsewhere. So:
 * stored order is honoured, unknown ids are dropped, repeats are ignored, and
 * **sections the layout has never heard of are appended** rather than lost. A
 * panel added in a later release therefore appears for someone with a saved
 * layout instead of silently vanishing.
 *
 * @param stored The layout as read from preferences.
 * @param ids The section ids the code defines, in canonical order.
 * @returns One entry per section, in the order they should render.
 */
export function reconcileLayout(
  stored: readonly PanelLayout[],
  ids: readonly string[]
): PanelLayout[] {
  const known = new Set(ids);
  const seen = new Set<string>();
  const panels: PanelLayout[] = [];

  for (const entry of stored) {
    if (!known.has(entry.id) || seen.has(entry.id)) continue;
    seen.add(entry.id);
    panels.push(normalise(entry));
  }

  for (const id of ids) {
    if (!seen.has(id)) panels.push(defaultPanelLayout(id));
  }

  return panels;
}

/**
 * Moves one item to a new index.
 *
 * An out-of-range index returns the list unchanged rather than throwing or
 * producing a hole: this is driven by a pointer, and a drag that ends somewhere
 * unexpected should do nothing rather than corrupt the arrangement.
 *
 * @param list The list to reorder.
 * @param from The index to move.
 * @param to The index to move it to.
 */
export function reorder<T>(list: readonly T[], from: number, to: number): T[] {
  const next = [...list];
  if (from < 0 || from >= next.length || to < 0 || to >= next.length || from === to) return next;

  const [moved] = next.splice(from, 1);
  if (moved === undefined) return [...list];

  next.splice(to, 0, moved);
  return next;
}

/**
 * Moves a panel by `delta` places, stopping at the ends.
 *
 * Deliberately does not wrap: "move up" on the first panel sending it to the
 * bottom would be a surprise, not a convenience.
 *
 * @param list The current layout.
 * @param id The panel to move.
 * @param delta `-1` for up, `1` for down.
 */
export function moveById(list: readonly PanelLayout[], id: string, delta: number): PanelLayout[] {
  const from = list.findIndex((panel) => panel.id === id);
  if (from === -1) return [...list];

  const to = from + delta;
  if (to < 0 || to >= list.length) return [...list];

  return reorder(list, from, to);
}

/**
 * Applies a change to one panel.
 *
 * @param list The current layout.
 * @param id The panel to change.
 * @param change The fields to set.
 */
export function updatePanel(
  list: readonly PanelLayout[],
  id: string,
  change: Partial<Omit<PanelLayout, 'id'>>
): PanelLayout[] {
  return list.map((panel) => (panel.id === id ? normalise({ ...panel, ...change }) : panel));
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `npm run test --workspace @osstat/ui -- panelLayout`
Expected: PASS, 20 tests.

- [ ] **Step 5: Typecheck and lint**

Run: `npm run typecheck --workspace @osstat/ui; npm run lint --workspace @osstat/ui`
Expected: both exit 0.

- [ ] **Step 6: Commit**

```bash
git add ui/src/lib/panelLayout.ts ui/src/lib/panelLayout.test.ts
git commit -m "feat(ui): add the Overview panel layout model

Order, size and visibility as plain data, with the rules that matter as
pure functions.

Reconciliation is the load-bearing part. A stored layout and a shipped
release will drift, so the stored list is never treated as the authority
on which panels exist: sections it does not mention are appended rather
than dropped, which is what stops a panel added in a later release from
vanishing for anyone with a saved arrangement.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 3: Store the layout beside the other preferences

`Preferences` gains one field. No storage key bump: `coercePreferences` already falls back field by field, so a record written by the current build simply has no panel list.

One type change is forced. `isAllowed<K extends keyof Preferences>` indexes `CHOICES[key]`, and `overviewPanels` has no `CHOICES` entry — it is a list, not a radio group. The constraint narrows to `keyof typeof CHOICES`.

**Files:**

- Modify: `ui/src/lib/preferences.ts:25-34` (`Preferences`), `:61-66` (`DEFAULT_PREFERENCES`), `:71-73` (`isAllowed`), `:86-105` (`coercePreferences`)
- Test: `ui/src/lib/preferences.test.ts`

**Interfaces:**

- Consumes: `coercePanelLayout`, `PanelLayout` from Task 2.
- Produces: `Preferences.overviewPanels: PanelLayout[]`. `usePreferences()` keeps its `[Preferences, (update: Partial<Preferences>) => void]` shape.

- [ ] **Step 1: Write the failing tests**

Append to `ui/src/lib/preferences.test.ts`:

```ts
describe('overviewPanels', () => {
  it('defaults to an empty list, which reconciliation fills from the sections', () => {
    expect(DEFAULT_PREFERENCES.overviewPanels).toEqual([]);
  });

  it('reads a stored layout back', () => {
    const panels = [{ id: 'cpu', span: 6, height: 'tall', hidden: false }];
    expect(coercePreferences({ overviewPanels: panels }).overviewPanels).toEqual(panels);
  });

  it('falls back to no layout without disturbing the other settings', () => {
    const result = coercePreferences({ overviewPanels: 'sideways', navigation: 'rail' });
    expect(result.overviewPanels).toEqual([]);
    expect(result.navigation).toBe('rail');
  });

  it('repairs a bad entry rather than discarding the whole layout', () => {
    const result = coercePreferences({
      overviewPanels: [
        { id: 'cpu', span: 99 },
        { id: 'gpu', span: 4 },
      ],
    });
    expect(result.overviewPanels).toEqual([
      { id: 'cpu', span: 12, height: 'normal', hidden: false },
      { id: 'gpu', span: 4, height: 'normal', hidden: false },
    ]);
  });

  it('survives a record written before this field existed', () => {
    // The upgrade path: no key bump, no migration.
    const old = { navigation: 'tabs', pageLayout: 'subTabs', refreshMs: 5000, historySeconds: 60 };
    const result = coercePreferences(old);
    expect(result.overviewPanels).toEqual([]);
    expect(result.navigation).toBe('tabs');
    expect(result.refreshMs).toBe(5000);
  });
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npm run test --workspace @osstat/ui -- preferences`
Expected: FAIL — `overviewPanels` is `undefined`.

- [ ] **Step 3: Add the field**

In `ui/src/lib/preferences.ts`, add to the imports:

```ts
import { coercePanelLayout, type PanelLayout } from './panelLayout';
```

Add to the `Preferences` interface, after `historySeconds`:

```ts
  /**
   * How the Overview panels are arranged.
   *
   * Empty means "never arranged"; the canonical section order is used, and
   * `reconcileLayout` fills this in from the sections that actually exist.
   */
  overviewPanels: PanelLayout[];
```

Add to `DEFAULT_PREFERENCES`:

```ts
  overviewPanels: [],
```

Change the `isAllowed` signature from `<K extends keyof Preferences>` to:

```ts
function isAllowed<K extends keyof typeof CHOICES>(
  key: K,
  value: unknown
): value is Preferences[K] {
```

Add to the object `coercePreferences` returns:

```ts
    // Not an `isAllowed` check: this is a list, not one of a fixed set of
    // choices, so it has its own coercion that repairs entries individually.
    overviewPanels: coercePanelLayout(candidate.overviewPanels),
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `npm run test --workspace @osstat/ui -- preferences`
Expected: PASS.

- [ ] **Step 5: Run the whole suite and typecheck**

Run: `npm run test --workspace @osstat/ui; npm run typecheck --workspace @osstat/ui`
Expected: both exit 0. If `App.test.tsx` fails on a missing `overviewPanels` in a preference fixture, add `overviewPanels: []` to that fixture.

- [ ] **Step 6: Commit**

```bash
git add ui/src/lib/preferences.ts ui/src/lib/preferences.test.ts ui/src/App.test.tsx
git commit -m "feat(ui): persist the Overview panel arrangement

No storage key bump and no migration: coercePreferences already falls
back field by field, so a record written before this field existed reads
back with an empty layout and every other setting intact.

isAllowed narrows to keyof typeof CHOICES. The panel list is a list
rather than one of a fixed set of choices, so it has its own coercion.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 4: The panel menu, and a header that can hold it

Today the whole section header is one `<button>`. A grip and a menu button cannot nest inside it — nested buttons are invalid HTML and behave inconsistently — so the header becomes a flex container with three siblings.

The menu is where every arrangement action lives. That is what lets dragging stay a pointer-only enhancement: no keyboard user is ever asked to perform one.

**Files:**

- Create: `ui/src/components/PanelMenu.tsx`
- Create: `ui/src/components/PanelMenu.test.tsx`
- Create: `ui/src/components/Panel.tsx` (moved out of `Section.tsx`)
- Modify: `ui/src/components/Section.tsx:11-71` (`SectionSpec` and `Collapsible` move out)

**Why `Panel.tsx` exists:** Task 5 has `SectionContainer` delegate to `PanelGrid`, and `PanelGrid` renders `Collapsible`. If both stayed in `Section.tsx` the two modules would import each other. Circular imports between component modules resolve unpredictably depending on which side is evaluated first, so `SectionSpec` and `Collapsible` move to their own module that both import from. `Section.tsx` re-exports `SectionSpec`, so every existing import site keeps working.

**Interfaces:**

- Consumes: `PanelSpan`, `PanelHeight`, `PANEL_SPANS`, `PANEL_HEIGHT_NAMES`, `SPAN_LABELS`, `HEIGHT_LABELS` from Task 2.
- Produces:
  - `interface PanelControls { span: PanelSpan; height: PanelHeight; canMoveUp: boolean; canMoveDown: boolean; showSize: boolean; onSpan: (span: PanelSpan) => void; onHeight: (height: PanelHeight) => void; onMove: (delta: -1 | 1) => void; onHide: () => void }`
  - `PanelMenu(props: { title: string } & PanelControls): React.JSX.Element`
  - `ui/src/components/Panel.tsx` exports `SectionSpec` (unchanged shape) and `Collapsible`, which gains two optional props: `controls?: PanelControls`, `grip?: ReactNode`. Both absent means today's behaviour exactly.
  - `ui/src/components/Section.tsx` re-exports `SectionSpec` and keeps `SectionContainer`.

- [ ] **Step 1: Write the failing tests**

Create `ui/src/components/PanelMenu.test.tsx`:

```tsx
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { PanelMenu, type PanelControls } from './PanelMenu';

function controls(overrides: Partial<PanelControls> = {}): PanelControls {
  return {
    span: 12,
    height: 'normal',
    canMoveUp: true,
    canMoveDown: true,
    showSize: true,
    onSpan: vi.fn(),
    onHeight: vi.fn(),
    onMove: vi.fn(),
    onHide: vi.fn(),
    ...overrides,
  };
}

function open(props: PanelControls): void {
  render(<PanelMenu title="CPU" {...props} />);
  fireEvent.click(screen.getByRole('button', { name: /arrange cpu/i }));
}

describe('PanelMenu', () => {
  it('stays closed until asked', () => {
    render(<PanelMenu title="CPU" {...controls()} />);
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });

  it('reports the current width so the choice is visible, not just remembered', () => {
    open(controls({ span: 6 }));
    expect(screen.getByRole('menuitemradio', { name: 'Half' })).toHaveAttribute(
      'aria-checked',
      'true'
    );
  });

  it('changes the width', () => {
    const props = controls();
    open(props);
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Third' }));
    expect(props.onSpan).toHaveBeenCalledWith(4);
  });

  it('changes the height', () => {
    const props = controls();
    open(props);
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Tall' }));
    expect(props.onHeight).toHaveBeenCalledWith('tall');
  });

  it('reorders without a mouse drag', () => {
    const props = controls();
    open(props);
    fireEvent.click(screen.getByRole('menuitem', { name: /move up/i }));
    expect(props.onMove).toHaveBeenCalledWith(-1);
  });

  it('disables moving past either end rather than silently doing nothing', () => {
    open(controls({ canMoveUp: false }));
    expect(screen.getByRole('menuitem', { name: /move up/i })).toBeDisabled();
  });

  it('hides the panel', () => {
    const props = controls();
    open(props);
    fireEvent.click(screen.getByRole('menuitem', { name: /hide/i }));
    expect(props.onHide).toHaveBeenCalled();
  });

  it('omits size when one section fills the pane', () => {
    // In sub-tabs layout width and height mean nothing, and offering a control
    // that does nothing is worse than omitting it.
    open(controls({ showSize: false }));
    expect(screen.queryByRole('menuitemradio', { name: 'Half' })).not.toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: /move up/i })).toBeInTheDocument();
  });

  it('closes on Escape', () => {
    open(controls());
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npm run test --workspace @osstat/ui -- PanelMenu`
Expected: FAIL — cannot resolve `./PanelMenu`.

- [ ] **Step 3: Write the menu**

Create `ui/src/components/PanelMenu.tsx`:

```tsx
/**
 * The per-panel arrangement menu.
 *
 * Every way of arranging a panel lives here, including the two — Move up and
 * Move down — that dragging also provides. That redundancy is the point:
 * because the menu is complete, dragging can stay a pointer-only enhancement
 * and no keyboard user is ever asked to perform one.
 */

import { useEffect, useId, useRef, useState } from 'react';

import {
  HEIGHT_LABELS,
  PANEL_HEIGHT_NAMES,
  PANEL_SPANS,
  SPAN_LABELS,
  type PanelHeight,
  type PanelSpan,
} from '../lib/panelLayout';

/** What a panel can be told to do about its own placement. */
export interface PanelControls {
  /** Current width. */
  span: PanelSpan;
  /** Current height. */
  height: PanelHeight;
  /** Whether there is anywhere above to move to. */
  canMoveUp: boolean;
  /** Whether there is anywhere below to move to. */
  canMoveDown: boolean;
  /** Whether width and height apply at all. False under the sub-tabs layout. */
  showSize: boolean;
  /** Sets the width. */
  onSpan: (span: PanelSpan) => void;
  /** Sets the height. */
  onHeight: (height: PanelHeight) => void;
  /** Moves the panel one place; `-1` is up. */
  onMove: (delta: -1 | 1) => void;
  /** Hides the panel. */
  onHide: () => void;
}

/** A group heading inside the menu. */
function Group({ children }: { children: string }): React.JSX.Element {
  return (
    <p className="px-3 pb-1 pt-2 text-[10px] uppercase tracking-wider text-neutral-600">
      {children}
    </p>
  );
}

/** One row of the menu. */
const ROW =
  'block w-full px-3 py-1.5 text-left text-xs hover:bg-white/[0.06] disabled:opacity-40 disabled:hover:bg-transparent';

/**
 * Renders the arrangement menu for one panel.
 *
 * @param props The panel's title, for labelling, and its controls.
 */
export function PanelMenu({
  title,
  span,
  height,
  canMoveUp,
  canMoveDown,
  showSize,
  onSpan,
  onHeight,
  onMove,
  onHide,
}: { title: string } & PanelControls): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const container = useRef<HTMLDivElement | null>(null);
  const menuId = useId();

  useEffect(() => {
    if (!open) return undefined;

    const dismiss = (event: Event): void => {
      if (event instanceof KeyboardEvent && event.key !== 'Escape') return;
      if (event.type === 'pointerdown' && event.target instanceof Node) {
        if (container.current?.contains(event.target) === true) return;
      }
      setOpen(false);
    };

    document.addEventListener('keydown', dismiss);
    document.addEventListener('pointerdown', dismiss);

    return () => {
      document.removeEventListener('keydown', dismiss);
      document.removeEventListener('pointerdown', dismiss);
    };
  }, [open]);

  return (
    <div ref={container} className="relative">
      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        aria-label={`Arrange ${title}`}
        onClick={() => {
          setOpen((current) => !current);
        }}
        className="rounded px-1.5 py-1 text-neutral-500 hover:bg-white/[0.06] hover:text-neutral-200"
      >
        <span aria-hidden="true">⋮</span>
      </button>

      {open && (
        <div
          id={menuId}
          role="menu"
          aria-label={`Arrange ${title}`}
          className="absolute right-0 z-20 mt-1 w-44 overflow-hidden rounded-lg border border-edge bg-surface-raised py-1 shadow-lg"
        >
          {showSize && (
            <>
              <Group>Width</Group>
              {PANEL_SPANS.map((candidate) => (
                <button
                  key={candidate}
                  type="button"
                  role="menuitemradio"
                  aria-checked={candidate === span}
                  className={`${ROW} ${candidate === span ? 'text-accent' : 'text-neutral-300'}`}
                  onClick={() => {
                    onSpan(candidate);
                    setOpen(false);
                  }}
                >
                  {SPAN_LABELS[candidate]}
                </button>
              ))}

              <Group>Height</Group>
              {PANEL_HEIGHT_NAMES.map((candidate) => (
                <button
                  key={candidate}
                  type="button"
                  role="menuitemradio"
                  aria-checked={candidate === height}
                  className={`${ROW} ${candidate === height ? 'text-accent' : 'text-neutral-300'}`}
                  onClick={() => {
                    onHeight(candidate);
                    setOpen(false);
                  }}
                >
                  {HEIGHT_LABELS[candidate]}
                </button>
              ))}
            </>
          )}

          <Group>Order</Group>
          <button
            type="button"
            role="menuitem"
            disabled={!canMoveUp}
            className={`${ROW} text-neutral-300`}
            onClick={() => {
              onMove(-1);
              setOpen(false);
            }}
          >
            Move up
          </button>
          <button
            type="button"
            role="menuitem"
            disabled={!canMoveDown}
            className={`${ROW} text-neutral-300`}
            onClick={() => {
              onMove(1);
              setOpen(false);
            }}
          >
            Move down
          </button>

          <div className="my-1 border-t border-edge" />
          <button
            type="button"
            role="menuitem"
            className={`${ROW} text-neutral-300`}
            onClick={() => {
              onHide();
              setOpen(false);
            }}
          >
            Hide this panel
          </button>
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 4: Run the menu tests**

Run: `npm run test --workspace @osstat/ui -- PanelMenu`
Expected: PASS.

- [ ] **Step 5: Move the section and its header into `Panel.tsx`**

Create `ui/src/components/Panel.tsx`. Move `SectionSpec` (`Section.tsx:11-21`) into it verbatim, then add the restructured header below it:

```tsx
/**
 * One page section, and the frame it is drawn in.
 *
 * Separate from `Section.tsx` because `SectionContainer` delegates to
 * `PanelGrid`, and `PanelGrid` renders this. Leaving both here would make those
 * two modules import each other, and a cycle between component modules resolves
 * differently depending on which side a bundler evaluates first.
 */

import { useId, useState, type ReactNode } from 'react';

import { PanelMenu, type PanelControls } from './PanelMenu';

/** A collapsible section. */
interface CollapsibleProps {
  /** The section to render. */
  section: SectionSpec;
  /** Whether it starts open. */
  defaultOpen: boolean;
  /** The arrangement menu's behaviour, when the section can be arranged. */
  controls?: PanelControls;
  /** The drag grip, when reordering is available. */
  grip?: ReactNode;
}

/**
 * Renders one collapsible section.
 *
 * The summary stays visible when collapsed. A section that hid its headline
 * figure when shut would make collapsing cost information rather than space,
 * which is the opposite of the point.
 *
 * The header is a flex container rather than a single button because a grip and
 * a menu button cannot nest inside one: nested buttons are invalid HTML and
 * behave inconsistently across browsers. The collapse target is the middle
 * child, so clicking the title still toggles and clicking the controls does not.
 */
export function Collapsible({
  section,
  defaultOpen,
  controls,
  grip,
}: CollapsibleProps): React.JSX.Element {
  const [open, setOpen] = useState(defaultOpen);
  const contentId = useId();

  return (
    <section className="overflow-hidden rounded-xl border border-edge bg-surface-raised">
      <h3 className="flex items-center gap-1 pr-1.5">
        {grip}

        <button
          type="button"
          aria-expanded={open}
          aria-controls={contentId}
          onClick={() => {
            setOpen((current) => !current);
          }}
          className="flex min-w-0 flex-1 items-center gap-2 px-2 py-2.5 text-left hover:bg-white/[0.03]"
        >
          <span aria-hidden="true" className="text-xs text-accent">
            {open ? '▾' : '▸'}
          </span>
          <span className="truncate text-sm font-semibold">{section.title}</span>
          {section.summary !== undefined && (
            <span className="ml-auto truncate font-mono text-sm text-neutral-300">
              {section.summary}
            </span>
          )}
        </button>

        {controls !== undefined && <PanelMenu title={section.title} {...controls} />}
      </h3>

      {open && (
        <div id={contentId} className="border-t border-edge/70 px-4 py-3">
          {section.content}
        </div>
      )}
    </section>
  );
}
```

- [ ] **Step 6: Leave `Section.tsx` holding only the container**

Delete `SectionSpec` and `Collapsible` from `ui/src/components/Section.tsx`, and add at the top:

```tsx
import { Collapsible } from './Panel';

export type { SectionSpec } from './Panel';
```

`SectionContainer` stays as it is for now; Task 5 rewrites it. Every existing `import { ..., type SectionSpec } from '../components/Section'` keeps resolving through the re-export.

- [ ] **Step 7: Run the whole suite**

Run: `npm run test --workspace @osstat/ui; npm run typecheck --workspace @osstat/ui; npm run lint --workspace @osstat/ui`
Expected: all exit 0. `SectionContainer` passes neither `controls` nor `grip`, so its behaviour is unchanged.

- [ ] **Step 8: Commit**

```bash
git add ui/src/components/PanelMenu.tsx ui/src/components/PanelMenu.test.tsx ui/src/components/Panel.tsx ui/src/components/Section.tsx
git commit -m "feat(ui): add the per-panel arrangement menu

Every way of arranging a panel lives in the menu, including the two that
dragging also provides. That redundancy is deliberate: because the menu
is complete, dragging can stay a pointer-only enhancement and no
keyboard user is ever asked to perform one.

The section header becomes a flex container. It was a single button, and
a grip and a menu button cannot nest inside one -- nested buttons are
invalid HTML and behave inconsistently across browsers.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 5: The grid, and Overview wired to it

The panels become a 12-column flow grid honouring each entry's span, height and hidden flag, with the narrow-window rule measured from the container rather than the viewport — the sidebar's width counts against the panels, and a media query cannot see that.

**Files:**

- Create: `ui/src/pages/overviewPanels.ts`
- Create: `ui/src/components/PanelGrid.tsx`
- Create: `ui/src/components/PanelGrid.test.tsx`
- Modify: `ui/src/components/Section.tsx:73-134` (`SectionContainerProps`, `SectionContainer`)
- Modify: `ui/src/pages/Overview.tsx:30-42` (`OverviewProps`), `:66-264` (`sectionsFor`), `:359-378` (`Overview`)
- Modify: `ui/src/App.tsx:117-126` (the `overview` case)

**Interfaces:**

- Consumes: everything from Task 2; `PanelControls` from Task 4.
- Produces:
  - `OVERVIEW_PANELS: readonly { id: string; title: string }[]` and `OVERVIEW_PANEL_IDS: readonly string[]` from `overviewPanels.ts`
  - `PanelGrid(props: { sections: SectionSpec[]; layout: PanelLayout[]; onLayoutChange: (next: PanelLayout[]) => void }): React.JSX.Element`
  - `SectionContainerProps` gains `layout?: PanelLayout[]` and `onLayoutChange?: (next: PanelLayout[]) => void`
  - `OverviewProps` gains `panels: PanelLayout[]` and `onPanelsChange: (next: PanelLayout[]) => void`

- [ ] **Step 1: Write the canonical panel list**

Create `ui/src/pages/overviewPanels.ts`:

```ts
/**
 * The Overview's panels, named in one place.
 *
 * Both the page that renders them and the Settings list that restores hidden
 * ones need to agree on which panels exist and what they are called. Two lists
 * would drift; this is the one.
 */

/** One Overview panel's identity. */
export interface OverviewPanel {
  /** Stable id, matching the `SectionSpec.id` the page builds. */
  id: string;
  /** Heading shown on the panel, its tab and the Settings list. */
  title: string;
}

/** Every Overview panel, in the order a fresh install shows them. */
export const OVERVIEW_PANELS: readonly OverviewPanel[] = [
  { id: 'cpu', title: 'CPU' },
  { id: 'memory', title: 'Memory' },
  { id: 'disks', title: 'Disks' },
  { id: 'network', title: 'Network' },
  { id: 'gpu', title: 'GPU' },
];

/** Every Overview panel id, in canonical order. */
export const OVERVIEW_PANEL_IDS: readonly string[] = OVERVIEW_PANELS.map((panel) => panel.id);
```

- [ ] **Step 2: Write the failing grid tests**

Create `ui/src/components/PanelGrid.test.tsx`:

```tsx
import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { PanelGrid } from './PanelGrid';
import { defaultPanelLayout, type PanelLayout } from '../lib/panelLayout';
import type { SectionSpec } from './Section';

const SECTIONS: SectionSpec[] = [
  { id: 'cpu', title: 'CPU', content: <p>cpu body</p> },
  { id: 'memory', title: 'Memory', content: <p>memory body</p> },
];

function layout(...entries: Array<Partial<PanelLayout> & { id: string }>): PanelLayout[] {
  return entries.map((entry) => ({ ...defaultPanelLayout(entry.id), ...entry }));
}

/** Reports a fixed content width, since jsdom lays nothing out. */
function widthIs(pixels: number): void {
  vi.stubGlobal(
    'ResizeObserver',
    class {
      constructor(private readonly callback: ResizeObserverCallback) {}
      observe(target: Element): void {
        this.callback(
          [{ target, contentRect: { width: pixels } } as unknown as ResizeObserverEntry],
          this as unknown as ResizeObserver
        );
      }
      unobserve(): void {}
      disconnect(): void {}
    }
  );
}

describe('PanelGrid', () => {
  beforeEach(() => {
    widthIs(1200);
  });

  it('renders panels in the layout order, not the section order', () => {
    render(
      <PanelGrid
        sections={SECTIONS}
        layout={layout({ id: 'memory' }, { id: 'cpu' })}
        onLayoutChange={vi.fn()}
      />
    );

    const headings = screen.getAllByRole('heading', { level: 3 });
    expect(headings.map((heading) => heading.textContent)).toEqual([
      expect.stringContaining('Memory'),
      expect.stringContaining('CPU'),
    ]);
  });

  it('gives each panel its stored width', () => {
    render(
      <PanelGrid
        sections={SECTIONS}
        layout={layout({ id: 'cpu', span: 4 }, { id: 'memory', span: 8 })}
        onLayoutChange={vi.fn()}
      />
    );

    expect(screen.getByTestId('panel-cpu')).toHaveStyle({ gridColumn: 'span 4' });
    expect(screen.getByTestId('panel-memory')).toHaveStyle({ gridColumn: 'span 8' });
  });

  it('does not render a hidden panel', () => {
    render(
      <PanelGrid
        sections={SECTIONS}
        layout={layout({ id: 'cpu', hidden: true }, { id: 'memory' })}
        onLayoutChange={vi.fn()}
      />
    );

    expect(screen.queryByText('cpu body')).not.toBeInTheDocument();
    expect(screen.getByText('memory body')).toBeInTheDocument();
  });

  it('says how to get panels back when every one is hidden', () => {
    render(
      <PanelGrid
        sections={SECTIONS}
        layout={layout({ id: 'cpu', hidden: true }, { id: 'memory', hidden: true })}
        onLayoutChange={vi.fn()}
      />
    );

    expect(screen.getByRole('status')).toHaveTextContent(/settings/i);
  });

  it('gives every panel the full row when there is not enough width for columns', () => {
    // The stored span is not modified -- it applies again when there is room.
    widthIs(700);
    render(
      <PanelGrid
        sections={SECTIONS}
        layout={layout({ id: 'cpu', span: 4 }, { id: 'memory', span: 4 })}
        onLayoutChange={vi.fn()}
      />
    );

    expect(screen.getByTestId('panel-cpu')).toHaveStyle({ gridColumn: 'span 12' });
  });
});
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `npm run test --workspace @osstat/ui -- PanelGrid`
Expected: FAIL — cannot resolve `./PanelGrid`.

- [ ] **Step 4: Write the grid**

Create `ui/src/components/PanelGrid.tsx`:

```tsx
/**
 * The Overview's panels, arranged.
 *
 * A twelve-column flow grid: panels wrap rather than holding absolute
 * positions, so an arrangement can never contain a hole the user has to tidy
 * up, and a window too narrow for columns reflows instead of clipping.
 */

import { useEffect, useRef, useState } from 'react';

import { Collapsible, type SectionSpec } from './Panel';
import type { PanelControls } from './PanelMenu';
import {
  moveById,
  NARROW_WIDTH,
  PANEL_HEIGHTS,
  updatePanel,
  type PanelLayout,
} from '../lib/panelLayout';

/** What the grid needs. */
export interface PanelGridProps {
  /** The sections available to render. */
  sections: SectionSpec[];
  /** The reconciled layout, in render order, including hidden panels. */
  layout: PanelLayout[];
  /** Applies an arrangement change. */
  onLayoutChange: (next: PanelLayout[]) => void;
}

/**
 * Reports an element's content width, remeasured as it changes.
 *
 * Measured from the container rather than the viewport because the sidebar's
 * width counts against the panels, and a media query cannot see that.
 *
 * @param target The element to measure.
 * @returns The last measured width, or `null` before the first measurement.
 */
function useContentWidth(target: React.RefObject<HTMLElement | null>): number | null {
  const [width, setWidth] = useState<number | null>(null);

  useEffect(() => {
    const element = target.current;
    if (element === null) return undefined;

    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (entry !== undefined) setWidth(entry.contentRect.width);
    });

    observer.observe(element);
    return () => {
      observer.disconnect();
    };
  }, [target]);

  return width;
}

/**
 * Renders the arranged panels.
 *
 * @param props The sections, the layout and the change handler.
 */
export function PanelGrid({ sections, layout, onLayoutChange }: PanelGridProps): React.JSX.Element {
  const container = useRef<HTMLDivElement | null>(null);
  const width = useContentWidth(container);

  // Before the first measurement, assume there is room. Assuming the opposite
  // would make every panel flash full-width on mount.
  const narrow = width !== null && width < NARROW_WIDTH;

  const visible = layout.filter((panel) => !panel.hidden);

  return (
    <div ref={container} className="grid grid-cols-12 items-start gap-2">
      {visible.length === 0 && (
        <p role="status" className="col-span-12 py-8 text-center text-sm text-neutral-500">
          Every panel is hidden. Bring them back from Settings › Panels.
        </p>
      )}

      {visible.map((panel, index) => {
        const section = sections.find((candidate) => candidate.id === panel.id);
        if (section === undefined) return null;

        const controls: PanelControls = {
          span: panel.span,
          height: panel.height,
          canMoveUp: index > 0,
          canMoveDown: index < visible.length - 1,
          showSize: true,
          onSpan: (span) => {
            onLayoutChange(updatePanel(layout, panel.id, { span }));
          },
          onHeight: (height) => {
            onLayoutChange(updatePanel(layout, panel.id, { height }));
          },
          onMove: (delta) => {
            onLayoutChange(moveById(layout, panel.id, delta));
          },
          onHide: () => {
            onLayoutChange(updatePanel(layout, panel.id, { hidden: true }));
          },
        };

        return (
          <div
            key={panel.id}
            data-testid={`panel-${panel.id}`}
            style={{ gridColumn: `span ${narrow ? 12 : panel.span}` }}
          >
            <Collapsible section={section} defaultOpen controls={controls} />
          </div>
        );
      })}
    </div>
  );
}

/** The chart height a named panel height is worth. */
export function chartHeightFor(layout: readonly PanelLayout[], id: string): number {
  const panel = layout.find((candidate) => candidate.id === id);
  return PANEL_HEIGHTS[panel?.height ?? 'normal'];
}
```

- [ ] **Step 5: Export `Collapsible` and route `SectionContainer` through the grid**

In `ui/src/components/Section.tsx`, change `function Collapsible(` to `export function Collapsible(`.

Replace `SectionContainerProps` and the body of `SectionContainer` (`:73-134`) with:

```tsx
/** How a page presents its sections. */
export interface SectionContainerProps {
  /** The sections, in canonical order. */
  sections: SectionSpec[];
  /** `onePage` arranges them in a grid; `subTabs` shows one at a time. */
  layout: 'onePage' | 'subTabs';
  /** The panel arrangement, when the page supports arranging. */
  panels?: PanelLayout[];
  /** Applies an arrangement change. */
  onPanelsChange?: (next: PanelLayout[]) => void;
}

/**
 * Presents a page's sections according to the layout preference.
 *
 * @param props The sections, the chosen layout, and the panel arrangement.
 */
export function SectionContainer({
  sections,
  layout,
  panels,
  onPanelsChange,
}: SectionContainerProps): React.JSX.Element {
  const [active, setActive] = useState(sections[0]?.id ?? '');

  // Ordered by the arrangement where there is one, so the sub-tab order is the
  // same order the grid uses. Hidden panels are hidden in both.
  const arranged =
    panels === undefined
      ? sections
      : panels
          .filter((panel) => !panel.hidden)
          .flatMap((panel) => sections.filter((section) => section.id === panel.id));

  if (layout === 'subTabs') {
    const current = arranged.find((section) => section.id === active) ?? arranged[0];

    // One menu, acting on the selected tab. Width and height are omitted:
    // one section fills the pane, so they would do nothing, and a control that
    // does nothing is worse than no control. Order still matters here — it is
    // the tab order — and so does hiding.
    const index = current === undefined ? -1 : arranged.findIndex((s) => s.id === current.id);

    return (
      <div className="flex flex-col gap-3">
        <div
          role="tablist"
          aria-label="Sections"
          className="flex items-center gap-1 border-b border-edge"
        >
          {arranged.map((section) => {
            const selected = section.id === current?.id;
            return (
              <button
                key={section.id}
                type="button"
                role="tab"
                aria-selected={selected}
                onClick={() => {
                  setActive(section.id);
                }}
                className={`-mb-px border-b-2 px-3 py-2 text-sm ${
                  selected
                    ? 'border-accent text-neutral-100'
                    : 'border-transparent text-neutral-400 hover:text-neutral-200'
                }`}
              >
                {section.title}
              </button>
            );
          })}

          {current !== undefined && panels !== undefined && onPanelsChange !== undefined && (
            <div className="ml-auto pb-1">
              <PanelMenu
                title={current.title}
                span={12}
                height="normal"
                canMoveUp={index > 0}
                canMoveDown={index >= 0 && index < arranged.length - 1}
                showSize={false}
                onSpan={() => {}}
                onHeight={() => {}}
                onMove={(delta) => {
                  onPanelsChange(moveById(panels, current.id, delta));
                }}
                onHide={() => {
                  const remaining = arranged.filter((section) => section.id !== current.id);
                  // Move off the tab about to disappear, or the panel would be
                  // hidden while still selected and the pane would go blank.
                  setActive(remaining[0]?.id ?? '');
                  onPanelsChange(updatePanel(panels, current.id, { hidden: true }));
                }}
              />
            </div>
          )}
        </div>

        {current !== undefined && (
          <div role="tabpanel" aria-label={current.title}>
            {current.content}
          </div>
        )}
      </div>
    );
  }

  if (panels !== undefined && onPanelsChange !== undefined) {
    return <PanelGrid sections={sections} layout={panels} onLayoutChange={onPanelsChange} />;
  }

  return (
    <div className="flex flex-col gap-2">
      {arranged.map((section) => (
        <Collapsible key={section.id} section={section} defaultOpen />
      ))}
    </div>
  );
}
```

Add to that file's imports:

```tsx
import { PanelGrid } from './PanelGrid';
import { PanelMenu } from './PanelMenu';
import { moveById, updatePanel, type PanelLayout } from '../lib/panelLayout';
```

- [ ] **Step 6: Wire Overview**

In `ui/src/pages/Overview.tsx`:

Add to the imports:

```tsx
import { chartHeightFor } from '../components/PanelGrid';
import { reconcileLayout, type PanelLayout } from '../lib/panelLayout';
import { OVERVIEW_PANEL_IDS } from './overviewPanels';
```

Add to `OverviewProps`:

```tsx
  /** The stored panel arrangement. */
  panels: PanelLayout[];
  /** Applies an arrangement change. */
  onPanelsChange: (next: PanelLayout[]) => void;
```

In `sectionsFor`, change the destructure on the first line to `const { system, samples, latest, gpus, panels } = props;` and add below it:

```ts
const reconciled = reconcileLayout(panels, OVERVIEW_PANEL_IDS);
const heightOf = (id: string): number => chartHeightFor(reconciled, id);
```

Then replace the four hard-coded chart heights: `height={120}` on the CPU chart becomes `height={heightOf('cpu')}`, `height={120}` on the memory chart becomes `height={heightOf('memory')}`, and `height={110}` on the throughput chart becomes `height={heightOf('network')}`. Leave the per-core heatmap's computed height alone — it is derived from the core count, not from a preference.

Replace the `Overview` function body's `SectionContainer` call with:

```tsx
<SectionContainer
  sections={sectionsFor(props)}
  layout={props.layout}
  panels={reconcileLayout(props.panels, OVERVIEW_PANEL_IDS)}
  onPanelsChange={props.onPanelsChange}
/>
```

- [ ] **Step 7: Wire App**

In `ui/src/App.tsx`, replace the `overview` case (`:117-126`) with:

```tsx
    case 'overview':
      return (
        <Overview
          system={system}
          samples={samples}
          latest={latest}
          gpus={gpus}
          layout={preferences.pageLayout}
          panels={preferences.overviewPanels}
          onPanelsChange={(overviewPanels) => {
            onPreferenceChange({ overviewPanels });
          }}
        />
      );
```

- [ ] **Step 8: Run everything**

Run: `npm run test --workspace @osstat/ui; npm run typecheck --workspace @osstat/ui; npm run lint --workspace @osstat/ui`
Expected: all exit 0.

- [ ] **Step 9: Commit**

```bash
git add ui/src/pages/overviewPanels.ts ui/src/components/PanelGrid.tsx ui/src/components/PanelGrid.test.tsx ui/src/components/Section.tsx ui/src/pages/Overview.tsx ui/src/App.tsx
git commit -m "feat(ui): arrange the Overview panels in a flow grid

Twelve columns, and panels wrap rather than holding absolute positions,
so an arrangement can never contain a hole the user has to tidy up.

The narrow-window rule measures the container, not the viewport: the
sidebar's width counts against the panels and a media query cannot see
that. Below 900px every panel takes the full row, without modifying the
stored span, which applies again as soon as there is room.

Sub-tabs share the arrangement, so tab order is grid order and a hidden
panel is hidden in both.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 6: Drag to reorder

The pointer-only half. Panel bodies stop receiving pointer events during a drag, because otherwise ECharts swallows the gesture the moment the pointer crosses a chart.

**Files:**

- Create: `ui/src/lib/useDragReorder.ts`
- Create: `ui/src/lib/useDragReorder.test.tsx`
- Modify: `ui/src/components/PanelGrid.tsx`

**Interfaces:**

- Consumes: `reorder` from Task 2.
- Produces: `useDragReorder(order: readonly string[], onReorder: (next: string[]) => void): DragReorder` where

```ts
interface DragReorder {
  draggingId: string | null;
  register: (id: string) => (element: HTMLElement | null) => void;
  gripProps: (id: string) => {
    onPointerDown: (event: React.PointerEvent) => void;
    role: 'button';
    tabIndex: -1;
    'aria-hidden': true;
  };
}
```

- [ ] **Step 1: Write the failing test**

Create `ui/src/lib/useDragReorder.test.tsx`:

```tsx
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { useDragReorder } from './useDragReorder';

/** A minimal consumer, so the hook is tested through its real interface. */
function Harness({ onReorder }: { onReorder: (next: string[]) => void }): React.JSX.Element {
  const drag = useDragReorder(['a', 'b', 'c'], onReorder);

  return (
    <div>
      {['a', 'b', 'c'].map((id, index) => (
        <div key={id} ref={drag.register(id)} data-testid={`row-${id}`}>
          <span data-testid={`grip-${id}`} {...drag.gripProps(id)} />
          <span>{drag.draggingId === id ? `${id} dragging` : id}</span>
          <span data-testid={`index-${id}`}>{index}</span>
        </div>
      ))}
    </div>
  );
}

/** jsdom lays nothing out, so rectangles are supplied. */
function stackRows(): void {
  for (const [id, top] of [
    ['a', 0],
    ['b', 100],
    ['c', 200],
  ] as const) {
    screen.getByTestId(`row-${id}`).getBoundingClientRect = () =>
      ({
        top,
        bottom: top + 100,
        left: 0,
        right: 400,
        height: 100,
        width: 400,
        x: 0,
        y: top,
      }) as DOMRect;
  }
}

describe('useDragReorder', () => {
  it('does nothing until a drag starts', () => {
    render(<Harness onReorder={vi.fn()} />);
    expect(screen.queryByText(/dragging/)).not.toBeInTheDocument();
  });

  it('marks the dragged panel while the pointer is down', () => {
    render(<Harness onReorder={vi.fn()} />);
    stackRows();
    fireEvent.pointerDown(screen.getByTestId('grip-a'), { clientY: 10, pointerId: 1 });
    expect(screen.getByText('a dragging')).toBeInTheDocument();
  });

  it('reorders on drop, by the row the pointer ended over', () => {
    const onReorder = vi.fn();
    render(<Harness onReorder={onReorder} />);
    stackRows();

    fireEvent.pointerDown(screen.getByTestId('grip-a'), { clientY: 10, pointerId: 1 });
    fireEvent.pointerMove(document, { clientY: 250, pointerId: 1 });
    fireEvent.pointerUp(document, { clientY: 250, pointerId: 1 });

    expect(onReorder).toHaveBeenCalledWith(['b', 'c', 'a']);
  });

  it('abandons the drag on Escape without reordering', () => {
    const onReorder = vi.fn();
    render(<Harness onReorder={onReorder} />);
    stackRows();

    fireEvent.pointerDown(screen.getByTestId('grip-a'), { clientY: 10, pointerId: 1 });
    fireEvent.pointerMove(document, { clientY: 250, pointerId: 1 });
    fireEvent.keyDown(document, { key: 'Escape' });

    expect(onReorder).not.toHaveBeenCalled();
    expect(screen.queryByText(/dragging/)).not.toBeInTheDocument();
  });

  it('does not report a reorder when the panel is dropped where it started', () => {
    const onReorder = vi.fn();
    render(<Harness onReorder={onReorder} />);
    stackRows();

    fireEvent.pointerDown(screen.getByTestId('grip-b'), { clientY: 110, pointerId: 1 });
    fireEvent.pointerUp(document, { clientY: 115, pointerId: 1 });

    expect(onReorder).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm run test --workspace @osstat/ui -- useDragReorder`
Expected: FAIL — cannot resolve `./useDragReorder`.

- [ ] **Step 3: Write the hook**

Create `ui/src/lib/useDragReorder.ts`:

```ts
/**
 * Reordering by dragging a grip.
 *
 * Written against pointer events rather than a drag library. The job is
 * reordering a handful of cards in a flow container, keyboard access is already
 * covered by the panel menu, and this follows `Chart.tsx`: hand-written because
 * a dependency would cost more than it saves.
 *
 * The gesture starts from a grip rather than anywhere on the panel because
 * panels hold charts with hover crosshairs — dragging to reorder and dragging
 * across a chart to read it would otherwise be the same gesture.
 */

import { useCallback, useEffect, useRef, useState } from 'react';

import { reorder } from './panelLayout';

/** What a consumer needs to wire up a draggable list. */
export interface DragReorder {
  /** The id currently being dragged, or `null`. */
  draggingId: string | null;
  /** Ref callback registering the element that represents `id`. */
  register: (id: string) => (element: HTMLElement | null) => void;
  /** Props for the grip that starts a drag of `id`. */
  gripProps: (id: string) => {
    onPointerDown: (event: React.PointerEvent) => void;
    role: 'button';
    tabIndex: -1;
    'aria-hidden': true;
  };
}

/**
 * Tracks a drag-to-reorder gesture.
 *
 * @param order The ids currently rendered, in render order.
 * @param onReorder Called with the new order when a drag ends somewhere new.
 * @returns The dragging state and the props that drive it.
 */
export function useDragReorder(
  order: readonly string[],
  onReorder: (next: string[]) => void
): DragReorder {
  const [draggingId, setDraggingId] = useState<string | null>(null);

  // Refs rather than state: these change during a gesture, and re-rendering on
  // every pointermove would be both wasteful and, for the element map, wrong.
  const elements = useRef(new Map<string, HTMLElement>());
  const latestOrder = useRef(order);
  const dropIndex = useRef<number | null>(null);

  useEffect(() => {
    latestOrder.current = order;
  }, [order]);

  const register = useCallback(
    (id: string) => (element: HTMLElement | null) => {
      if (element === null) elements.current.delete(id);
      else elements.current.set(id, element);
    },
    []
  );

  useEffect(() => {
    if (draggingId === null) return undefined;

    /** The index whose row currently contains `y`. */
    const indexAt = (y: number): number | null => {
      for (const [index, id] of latestOrder.current.entries()) {
        const rect = elements.current.get(id)?.getBoundingClientRect();
        if (rect === undefined) continue;
        if (y >= rect.top && y <= rect.bottom) return index;
      }
      return null;
    };

    const move = (event: PointerEvent): void => {
      dropIndex.current = indexAt(event.clientY);
    };

    const finish = (): void => {
      const from = latestOrder.current.indexOf(draggingId);
      const to = dropIndex.current;

      setDraggingId(null);
      dropIndex.current = null;

      if (to === null || from === -1 || from === to) return;
      onReorder(reorder(latestOrder.current, from, to));
    };

    const cancel = (event: KeyboardEvent): void => {
      if (event.key !== 'Escape') return;
      setDraggingId(null);
      dropIndex.current = null;
    };

    document.addEventListener('pointermove', move);
    document.addEventListener('pointerup', finish);
    document.addEventListener('keydown', cancel);

    return () => {
      document.removeEventListener('pointermove', move);
      document.removeEventListener('pointerup', finish);
      document.removeEventListener('keydown', cancel);
    };
  }, [draggingId, onReorder]);

  const gripProps = useCallback(
    (id: string) =>
      ({
        onPointerDown: (event: React.PointerEvent) => {
          event.preventDefault();
          dropIndex.current = latestOrder.current.indexOf(id);
          setDraggingId(id);
        },
        role: 'button',
        tabIndex: -1,
        'aria-hidden': true,
      }) as const,
    []
  );

  return { draggingId, register, gripProps };
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `npm run test --workspace @osstat/ui -- useDragReorder`
Expected: PASS.

- [ ] **Step 5: Wire the grip into the grid**

In `ui/src/components/PanelGrid.tsx`, add to the imports:

```tsx
import { useDragReorder } from '../lib/useDragReorder';
```

Inside `PanelGrid`, after `const visible = ...`, add:

```tsx
const visibleIds = visible.map((panel) => panel.id);

const drag = useDragReorder(visibleIds, (next) => {
  // Reordering the visible panels must not disturb the hidden ones, so the
  // new order is applied to the visible slots of the full layout.
  let cursor = 0;
  onLayoutChange(
    layout.map((panel) => {
      if (panel.hidden) return panel;
      const id = next[cursor++];
      return layout.find((candidate) => candidate.id === id) ?? panel;
    })
  );
});
```

Change the panel wrapper `<div>` to carry the ref, the drag state and the pointer-events guard, and pass a grip to `Collapsible`:

```tsx
return (
  <div
    key={panel.id}
    ref={drag.register(panel.id)}
    data-testid={`panel-${panel.id}`}
    style={{ gridColumn: `span ${narrow ? 12 : panel.span}` }}
    className={drag.draggingId === panel.id ? 'opacity-50' : undefined}
  >
    <div
      // Charts capture the pointer for their crosshair. During a drag
      // that would swallow the gesture the moment the pointer crossed
      // one, so bodies stop listening until the drag ends.
      className={drag.draggingId === null ? undefined : '[&_[id]]:pointer-events-none'}
    >
      <Collapsible
        section={section}
        defaultOpen
        controls={controls}
        grip={
          <span
            {...drag.gripProps(panel.id)}
            title={`Drag to reorder ${section.title}`}
            className="cursor-grab select-none px-1.5 py-2 text-neutral-600 hover:text-neutral-300"
          >
            ⠿
          </span>
        }
      />
    </div>
  </div>
);
```

- [ ] **Step 6: Run everything**

Run: `npm run test --workspace @osstat/ui; npm run typecheck --workspace @osstat/ui; npm run lint --workspace @osstat/ui`
Expected: all exit 0.

- [ ] **Step 7: Commit**

```bash
git add ui/src/lib/useDragReorder.ts ui/src/lib/useDragReorder.test.tsx ui/src/components/PanelGrid.tsx
git commit -m "feat(ui): reorder Overview panels by dragging a grip

Pointer events rather than a drag library: the job is reordering a
handful of cards, keyboard access is already covered by the panel menu,
and this follows Chart.tsx.

The gesture starts from a grip rather than anywhere on the panel because
panels hold charts with hover crosshairs -- dragging to reorder and
dragging across a chart to read it would otherwise be the same gesture.
Bodies stop receiving pointer events for the duration for the same
reason.

Reordering the visible panels leaves hidden ones where they are.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 7: Settings — restore hidden panels, and reset

The way back. Without this, hiding a panel from its own menu is one-way.

**Files:**

- Modify: `ui/src/pages/Settings.tsx`
- Test: `ui/src/pages/Settings.test.tsx` (create if absent)

**Interfaces:**

- Consumes: `OVERVIEW_PANELS` (Task 5), `reconcileLayout`, `updatePanel` (Task 2), `Preferences.overviewPanels` (Task 3).
- Produces: no new exports; `SettingsProps` is unchanged.

- [ ] **Step 1: Write the failing tests**

Create (or append to) `ui/src/pages/Settings.test.tsx`:

```tsx
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { Settings } from './Settings';
import { DEFAULT_PREFERENCES, type Preferences } from '../lib/preferences';

function prefs(overrides: Partial<Preferences> = {}): Preferences {
  return { ...DEFAULT_PREFERENCES, ...overrides };
}

describe('Settings › Panels', () => {
  it('lists every Overview panel', () => {
    render(<Settings preferences={prefs()} onChange={vi.fn()} />);
    for (const title of ['CPU', 'Memory', 'Disks', 'Network', 'GPU']) {
      expect(screen.getByRole('checkbox', { name: title })).toBeInTheDocument();
    }
  });

  it('shows a panel as ticked when it is visible', () => {
    render(<Settings preferences={prefs()} onChange={vi.fn()} />);
    expect(screen.getByRole('checkbox', { name: 'GPU' })).toBeChecked();
  });

  it('shows a hidden panel as unticked', () => {
    const preferences = prefs({
      overviewPanels: [{ id: 'gpu', span: 12, height: 'normal', hidden: true }],
    });
    render(<Settings preferences={preferences} onChange={vi.fn()} />);
    expect(screen.getByRole('checkbox', { name: 'GPU' })).not.toBeChecked();
  });

  it('brings a hidden panel back', () => {
    const onChange = vi.fn();
    const preferences = prefs({
      overviewPanels: [{ id: 'gpu', span: 12, height: 'normal', hidden: true }],
    });
    render(<Settings preferences={preferences} onChange={onChange} />);

    fireEvent.click(screen.getByRole('checkbox', { name: 'GPU' }));

    const [update] = onChange.mock.calls[0] as [Partial<Preferences>];
    expect(update.overviewPanels?.find((panel) => panel.id === 'gpu')?.hidden).toBe(false);
  });

  it('resets order, sizes and hidden flags together', () => {
    const onChange = vi.fn();
    const preferences = prefs({
      overviewPanels: [
        { id: 'gpu', span: 4, height: 'tall', hidden: true },
        { id: 'cpu', span: 6, height: 'short', hidden: false },
      ],
    });
    render(<Settings preferences={preferences} onChange={onChange} />);

    fireEvent.click(screen.getByRole('button', { name: /reset overview layout/i }));

    expect(onChange).toHaveBeenCalledWith({ overviewPanels: [] });
  });
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npm run test --workspace @osstat/ui -- Settings`
Expected: FAIL — no checkbox named CPU.

- [ ] **Step 3: Add the Panels block**

In `ui/src/pages/Settings.tsx`, add to the imports:

```tsx
import { reconcileLayout, updatePanel } from '../lib/panelLayout';
import { OVERVIEW_PANELS, OVERVIEW_PANEL_IDS } from './overviewPanels';
```

Add this component above `Settings`:

```tsx
/** The list of Overview panels, and the way back from hiding one. */
function Panels({ preferences, onChange }: SettingsProps): React.JSX.Element {
  const panels = reconcileLayout(preferences.overviewPanels, OVERVIEW_PANEL_IDS);

  return (
    <div className="flex flex-col gap-2 border-b border-edge px-4 py-3 last:border-b-0">
      <div>
        <p className="text-sm">Panels</p>
        <p className="text-xs text-neutral-500">
          Which sections the Overview shows. Order and size are set from each panel&rsquo;s own
          menu.
        </p>
      </div>

      <div className="flex flex-col gap-1">
        {OVERVIEW_PANELS.map((panel) => {
          const hidden = panels.find((entry) => entry.id === panel.id)?.hidden ?? false;
          return (
            <label key={panel.id} className="flex items-center gap-2 text-xs text-neutral-300">
              <input
                type="checkbox"
                checked={!hidden}
                aria-label={panel.title}
                onChange={() => {
                  onChange({ overviewPanels: updatePanel(panels, panel.id, { hidden: !hidden }) });
                }}
                className="accent-accent"
              />
              {panel.title}
            </label>
          );
        })}
      </div>

      <div>
        <button
          type="button"
          onClick={() => {
            // Empty rather than a rebuilt default list: reconciliation fills it
            // from the sections that exist, which is the one place that knows.
            onChange({ overviewPanels: [] });
          }}
          className="rounded-md border border-edge px-3 py-1 text-xs text-neutral-400 hover:bg-white/[0.04] hover:text-neutral-200"
        >
          Reset Overview layout
        </button>
      </div>
    </div>
  );
}
```

Add `<Panels preferences={preferences} onChange={onChange} />` inside the settings card, after the `Page layout` choice.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `npm run test --workspace @osstat/ui -- Settings`
Expected: PASS.

- [ ] **Step 5: Verify Phase A end to end**

Run: `npm run test --workspace @osstat/ui; npm run typecheck --workspace @osstat/ui; npm run lint --workspace @osstat/ui`
Then run the app: `npm run dev`

Check by hand:

1. Drag the GPU panel above CPU by its ⠿ grip. Reload — the order persists.
2. Set CPU to Half and Memory to Half. They sit side by side.
3. Set CPU to Tall. Its chart grows.
4. Hide Disks from its ⋮ menu. Settings › Panels shows it unticked; tick it and it returns.
5. Reset Overview layout restores the original order and sizes.
6. Switch Page layout to Sub-tabs. The tab order matches the grid order; hidden panels have no tab.
7. Narrow the window to its 900px minimum with the sidebar showing. Panels go full width.

- [ ] **Step 6: Commit**

```bash
git add ui/src/pages/Settings.tsx ui/src/pages/Settings.test.tsx
git commit -m "feat(ui): restore hidden panels and reset the Overview layout

The way back from hiding a panel, which is what makes hiding safe to try.
Reset clears order, sizes and hidden flags together, and clears them to
an empty list rather than a rebuilt default -- reconciliation fills it
from the sections that exist, which is the one place that knows.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 8: The sampler learns to run in the background

`Control.paused: bool` cannot express "hidden but still feeding a tooltip". It becomes a three-state `Activity`.

**Files:**

- Modify: `src-tauri/src/sampler.rs:53-57` (interval constants), `:67-72` (`Control`), `:97-186` (`Sampler`), `:189-253` (`run`, `wait_for_tick`), `:283-351` (tests)
- Modify: `src-tauri/src/lib.rs:40-48` (the window event handler)
- Modify: `src-tauri/src/commands.rs:112-116` (`set_sampling_paused`)

**Interfaces:**

- Consumes: nothing from earlier tasks.
- Produces:
  - `pub enum Activity { Foreground, Background, Paused }` in `sampler`
  - `pub const BACKGROUND_INTERVAL: Duration`
  - `pub fn activity_for(visible: bool, minimized: bool, user_paused: bool) -> Activity`
  - `Sampler::set_window_state(&self, visible: bool, minimized: bool)`
  - `Sampler::activity(&self) -> Activity`
  - `Sampler::set_paused` and `Sampler::is_paused` keep their signatures

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module in `src-tauri/src/sampler.rs`:

```rust
    #[test]
    fn a_visible_window_samples_at_the_users_rate() {
        assert_eq!(activity_for(true, false, false), Activity::Foreground);
    }

    #[test]
    fn a_hidden_or_minimised_window_drops_to_the_background() {
        assert_eq!(activity_for(false, false, false), Activity::Background);
        assert_eq!(activity_for(true, true, false), Activity::Background);
        assert_eq!(activity_for(false, true, false), Activity::Background);
    }

    #[test]
    fn an_explicit_pause_beats_every_window_state() {
        // The user chose "Paused" in Settings. Restoring the window must not
        // quietly start sampling again.
        for (visible, minimized) in [(true, false), (false, false), (true, true)] {
            assert_eq!(activity_for(visible, minimized, true), Activity::Paused);
        }
    }

    #[test]
    fn the_background_tick_is_slower_than_anything_the_ui_offers() {
        assert!(BACKGROUND_INTERVAL > Duration::from_millis(5_000));
    }

    #[test]
    fn the_background_interval_replaces_the_users_rate_rather_than_scaling_it() {
        assert_eq!(
            effective_interval(Activity::Background, Duration::from_millis(1_000)),
            BACKGROUND_INTERVAL
        );
        assert_eq!(
            effective_interval(Activity::Foreground, Duration::from_millis(1_000)),
            Duration::from_millis(1_000)
        );
    }

    #[test]
    fn only_the_foreground_reads_the_process_table() {
        // The expensive half: 23.8 ms of a ~24 ms tick, and nothing in a tray
        // tooltip needs it.
        assert!(Activity::Foreground.reads_processes());
        assert!(!Activity::Background.reads_processes());
        assert!(!Activity::Paused.reads_processes());
    }

    #[test]
    fn only_the_foreground_emits_to_a_webview_that_can_see_it() {
        assert!(Activity::Foreground.emits());
        assert!(!Activity::Background.emits());
        assert!(!Activity::Paused.emits());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p osstat sampler`
Expected: FAIL — `activity_for` and `Activity` are not defined.

- [ ] **Step 3: Add the activity states**

In `src-tauri/src/sampler.rs`, add after the `MIN_INTERVAL` constant:

```rust
/// How often the sampler ticks while the window cannot be seen.
///
/// Slower than anything the UI offers, because the only thing it feeds is a
/// tray tooltip. It is deliberately not configurable: a setting for it would be
/// a knob whose only effect is how much battery osstat burns while nobody is
/// looking.
pub const BACKGROUND_INTERVAL: Duration = Duration::from_secs(5);

/// What the sampler is currently doing, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    /// The window is visible: full rate, everything measured, events emitted.
    Foreground,
    /// The window cannot be seen: slow rate, metrics only, no events.
    Background,
    /// The user asked for sampling to stop.
    Paused,
}

impl Activity {
    /// Whether this state reads the process table.
    ///
    /// Only the foreground does. The process read is the expensive half of a
    /// tick and nothing a hidden app displays needs it.
    #[must_use]
    pub const fn reads_processes(self) -> bool {
        matches!(self, Self::Foreground)
    }

    /// Whether this state emits events to the webview.
    ///
    /// Only the foreground does. Delivering IPC payloads to a hidden webview is
    /// work nobody can observe.
    #[must_use]
    pub const fn emits(self) -> bool {
        matches!(self, Self::Foreground)
    }

    /// Whether this state measures anything at all.
    #[must_use]
    pub const fn samples(self) -> bool {
        !matches!(self, Self::Paused)
    }
}

/// Decides what the sampler should be doing.
///
/// An explicit pause always wins: someone who chose "Paused" in Settings should
/// not find sampling quietly resumed by restoring the window.
#[must_use]
pub const fn activity_for(visible: bool, minimized: bool, user_paused: bool) -> Activity {
    if user_paused {
        Activity::Paused
    } else if !visible || minimized {
        Activity::Background
    } else {
        Activity::Foreground
    }
}

/// The tick length a given activity actually uses.
const fn effective_interval(activity: Activity, foreground: Duration) -> Duration {
    match activity {
        Activity::Background => BACKGROUND_INTERVAL,
        _ => foreground,
    }
}
```

- [ ] **Step 4: Rework `Control` and the loop**

Replace the `Control` struct with:

```rust
/// Knobs the UI can turn, and the signal that they have turned.
struct Control {
    /// The tick the user asked for, used in the foreground.
    interval: Duration,
    /// Whether the user has explicitly paused sampling.
    user_paused: bool,
    /// Whether the window is currently on screen.
    visible: bool,
    /// Whether the window is minimised.
    minimized: bool,
    stopping: bool,
}

impl Control {
    /// What the sampler should be doing given the current knobs.
    const fn activity(&self) -> Activity {
        activity_for(self.visible, self.minimized, self.user_paused)
    }
}
```

In `Sampler::start`, replace the `Control` initialiser with:

```rust
            control: Mutex::new(Control {
                interval: clamp_interval(interval),
                user_paused: false,
                visible: true,
                minimized: false,
                stopping: false,
            }),
```

Replace `set_interval`, `set_paused` and `is_paused` with:

```rust
    /// Changes the foreground tick interval, taking effect immediately.
    ///
    /// Out-of-range values are clamped rather than rejected: this arrives from
    /// a preference that a stale or hand-edited client could get wrong, and a
    /// clamped tick is better than a failed command or a busy loop.
    pub fn set_interval(&self, interval: Duration) {
        let mut control = lock(&self.shared.control);
        control.interval = clamp_interval(interval);
        control.user_paused = false;
        self.shared.changed.notify_all();
    }

    /// Suspends or resumes sampling at the user's explicit request.
    pub fn set_paused(&self, paused: bool) {
        lock(&self.shared.control).user_paused = paused;
        self.shared.changed.notify_all();
    }

    /// Whether sampling is suspended at the user's explicit request.
    #[must_use]
    pub fn is_paused(&self) -> bool {
        lock(&self.shared.control).user_paused
    }

    /// Records where the window is, which decides foreground versus background.
    pub fn set_window_state(&self, visible: bool, minimized: bool) {
        let mut control = lock(&self.shared.control);
        control.visible = visible;
        control.minimized = minimized;
        self.shared.changed.notify_all();
    }

    /// What the sampler is currently doing.
    #[must_use]
    pub fn activity(&self) -> Activity {
        lock(&self.shared.control).activity()
    }
```

Replace `run`'s loop body and `wait_for_tick` with:

```rust
    loop {
        let Some(activity) = wait_for_tick(shared) else {
            return;
        };

        let Ok(mut sample) = source.sample() else {
            continue;
        };
        sample.gpus = gpus.measure().unwrap_or_default();

        // The process table is read only in the foreground. It is the expensive
        // half of a tick, and nothing a hidden window shows depends on it.
        if activity.reads_processes() {
            let processes = source.processes().unwrap_or_default();
            let diff = diff_processes(&previous, &processes);

            write(&shared.snapshot).processes.clone_from(&processes);
            previous = processes;

            if activity.emits() && !diff.is_empty() {
                let _ = app.emit(PROCESSES_EVENT, &diff);
            }
        }

        write(&shared.snapshot).history.push(sample.clone());

        if activity.emits() {
            let _ = app.emit(METRICS_EVENT, &sample);
        }
    }
```

```rust
/// Sleeps until the next tick is due.
///
/// Returns the activity the tick should run as, or `None` when the sampler
/// should stop. While paused it waits without a timeout, so a paused sampler
/// costs nothing at all rather than costing a wakeup per interval.
fn wait_for_tick(shared: &Arc<Shared>) -> Option<Activity> {
    let mut control = lock(&shared.control);

    let activity = loop {
        if control.stopping {
            return None;
        }
        let activity = control.activity();
        if activity.samples() {
            break activity;
        }
        control = shared
            .changed
            .wait(control)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    };

    let interval = effective_interval(activity, control.interval);
    let (control, _) = shared
        .changed
        .wait_timeout(control, interval)
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    if control.stopping {
        None
    } else {
        // Re-read: the window may have moved between states during the sleep.
        Some(control.activity())
    }
}
```

- [ ] **Step 5: Update the window event handler**

In `src-tauri/src/lib.rs`, replace the `on_window_event` closure with:

```rust
        .on_window_event(|window, event| {
            // Both signals matter now: minimising and hiding to the tray are
            // different events, and both mean nobody can see the window. See
            // the module docs on `sampler` for why focus is still the wrong
            // signal to use.
            if matches!(event, WindowEvent::Resized(_) | WindowEvent::Focused(_))
                && let Some(sampler) = window.try_state::<Sampler>()
            {
                sampler.set_window_state(
                    window.is_visible().unwrap_or(true),
                    window.is_minimized().unwrap_or(false),
                );
            }
        })
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p osstat`
Expected: PASS. If the pre-existing `is_paused` test fails, it is asserting on the old field name — update it to use `set_paused`/`is_paused`, whose behaviour is unchanged.

- [ ] **Step 7: Lint**

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings; cargo fmt --all --check`
Expected: both exit 0.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/sampler.rs src-tauri/src/lib.rs
git commit -m "feat(core): give the sampler a background activity state

paused: bool could not express \"hidden but still feeding a tooltip\", so
it becomes three states. Background ticks every 5 s, reads metrics only
and emits nothing.

The process read is skipped because it is the expensive half -- 23.8 ms
of a ~24 ms tick -- and nothing a hidden window shows needs it. Events
are skipped because delivering IPC payloads to a hidden webview is work
nobody can observe.

An explicit pause beats every window state: someone who chose Paused in
Settings should not find sampling quietly resumed by restoring the
window.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 9: Close behaviour, decided in Rust

The close handler must decide synchronously, so the choice cannot be fetched from the webview at close time. Rust holds it; Settings writes it; the front-end replays it on startup the way it already replays the refresh interval.

**Files:**

- Create: `src-tauri/src/window_state.rs`
- Modify: `src-tauri/src/lib.rs` (module declaration, managed state, `CloseRequested` handling, handler list)
- Modify: `src-tauri/src/commands.rs` (new command)
- Modify: `ui/src/lib/ipc.ts` (`COMMANDS`, new function)
- Modify: `ui/src/lib/preferences.ts` (new setting)

**Interfaces:**

- Consumes: nothing from earlier tasks.
- Produces:
  - `pub enum CloseBehaviour { Hide, Quit }` with `Default` = `Hide`, serialised camelCase as `"hide"` / `"quit"`, exported by `ts-rs`
  - `pub struct CloseSetting(Mutex<CloseBehaviour>)` with `get(&self) -> CloseBehaviour` and `set(&self, behaviour: CloseBehaviour)`
  - Command `set_close_behaviour(state: State<'_, CloseSetting>, behaviour: CloseBehaviour)`
  - `setCloseBehaviour(behaviour: CloseBehaviour): Promise<void>` in `ipc.ts`
  - `Preferences.closeBehaviour: 'hide' | 'quit'`, default `'hide'`

- [ ] **Step 1: Write the failing Rust tests**

Create `src-tauri/src/window_state.rs` with only the tests first:

```rust
//! What the window does when it is closed.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hiding_is_the_default() {
        assert_eq!(CloseBehaviour::default(), CloseBehaviour::Hide);
        assert_eq!(CloseSetting::default().get(), CloseBehaviour::Hide);
    }

    #[test]
    fn a_change_survives_a_round_trip() {
        let setting = CloseSetting::default();
        setting.set(CloseBehaviour::Quit);
        assert_eq!(setting.get(), CloseBehaviour::Quit);
    }

    #[test]
    fn serialises_as_the_strings_the_front_end_stores() {
        assert_eq!(
            serde_json::to_string(&CloseBehaviour::Hide).unwrap(),
            "\"hide\""
        );
        assert_eq!(
            serde_json::to_string(&CloseBehaviour::Quit).unwrap(),
            "\"quit\""
        );
    }

    #[test]
    fn only_hiding_keeps_the_app_running() {
        assert!(CloseBehaviour::Hide.hides());
        assert!(!CloseBehaviour::Quit.hides());
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p osstat window_state`
Expected: FAIL — `CloseBehaviour` is not defined. (Add `pub mod window_state;` to `src-tauri/src/lib.rs` first, or the file is not compiled at all.)

- [ ] **Step 3: Write the implementation**

Put this above the `tests` module in `src-tauri/src/window_state.rs`:

```rust
//! What the window does when it is closed.
//!
//! The decision is held here, in Rust, rather than fetched from the webview
//! when the close arrives: `CloseRequested` must be answered synchronously
//! inside the event handler, and there is no opportunity to ask anything.
//!
//! That also makes the behaviour robust in the case that matters. A
//! confirmation dialog would have to be drawn by the webview that is about to
//! disappear; if it ever failed to draw, the close button would silently do
//! nothing and osstat would be dismissible only through its tray icon. Reading
//! a value already in memory cannot get stuck that way.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// What closing the window does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub enum CloseBehaviour {
    /// Keep running in the notification area.
    #[default]
    Hide,
    /// Exit.
    Quit,
}

impl CloseBehaviour {
    /// Whether closing should hide the window instead of exiting.
    #[must_use]
    pub const fn hides(self) -> bool {
        matches!(self, Self::Hide)
    }
}

/// The current close behaviour, held as Tauri managed state.
#[derive(Debug, Default)]
pub struct CloseSetting(Mutex<CloseBehaviour>);

impl CloseSetting {
    /// The current behaviour.
    #[must_use]
    pub fn get(&self) -> CloseBehaviour {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Replaces the behaviour.
    pub fn set(&self, behaviour: CloseBehaviour) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = behaviour;
    }
}
```

Add `#![allow(clippy::unwrap_used, clippy::expect_used)]` at the top of the `tests` module, matching the other test modules in this crate.

- [ ] **Step 4: Add the command and wire the handler**

In `src-tauri/src/commands.rs`, add to the imports `use crate::window_state::{CloseBehaviour, CloseSetting};` and add:

```rust
/// Sets what closing the window does.
///
/// Held in Rust because `CloseRequested` must be answered synchronously; see
/// [`crate::window_state`].
#[tauri::command]
pub fn set_close_behaviour(setting: State<'_, CloseSetting>, behaviour: CloseBehaviour) {
    setting.set(behaviour);
}
```

In `src-tauri/src/lib.rs`: add `pub mod window_state;` beside the other module declarations, add `use crate::window_state::CloseSetting;`, register `app.manage(CloseSetting::default());` in `setup`, add `commands::set_close_behaviour` to the `generate_handler!` list, and extend the window event handler:

```rust
            if let WindowEvent::CloseRequested { api, .. } = event
                && window
                    .try_state::<CloseSetting>()
                    .is_some_and(|setting| setting.get().hides())
            {
                api.prevent_close();
                let _ = window.hide();
                if let Some(sampler) = window.try_state::<Sampler>() {
                    sampler.set_window_state(false, false);
                }
            }
```

- [ ] **Step 5: Regenerate the bindings**

Run: `just bindings`
Expected: `ui/src/bindings/CloseBehaviour.ts` appears.

- [ ] **Step 6: Add the front-end edge**

In `ui/src/lib/ipc.ts`, add `setCloseBehaviour: 'set_close_behaviour',` to `COMMANDS`, add `import type { CloseBehaviour } from '../bindings/CloseBehaviour';`, and add:

```ts
/**
 * Tells the backend what closing the window should do.
 *
 * The decision has to be made synchronously inside Rust's close handler, so it
 * is pushed ahead of time rather than asked for at close time.
 *
 * @param behaviour Whether closing hides the window or exits.
 */
export async function setCloseBehaviour(behaviour: CloseBehaviour): Promise<void> {
  await invoke(COMMANDS.setCloseBehaviour, { behaviour });
}
```

In `ui/src/lib/preferences.ts`, add `closeBehaviour: CloseBehaviour;` to `Preferences` (importing the binding type), add to `CHOICES`:

```ts
  closeBehaviour: [
    { value: 'hide', label: 'Hide to the notification area' },
    { value: 'quit', label: 'Quit osstat' },
  ],
```

add `closeBehaviour: 'hide',` to `DEFAULT_PREFERENCES`, and add to `coercePreferences`'s returned object:

```ts
    closeBehaviour: isAllowed('closeBehaviour', candidate.closeBehaviour)
      ? candidate.closeBehaviour
      : DEFAULT_PREFERENCES.closeBehaviour,
```

In `ui/src/App.tsx`, replay it on startup and on change, beside the existing interval effect:

```tsx
useEffect(() => {
  // Replayed for the same reason as the interval: Rust holds the value it
  // needs at close time, and the front-end is where the preference lives.
  setCloseBehaviour(preferences.closeBehaviour).catch(() => {});
}, [preferences.closeBehaviour]);
```

- [ ] **Step 7: Add the Settings control**

In `ui/src/pages/Settings.tsx`, add after the `History window` choice:

```tsx
<Choice
  label="When I close the window"
  description="osstat can keep running in the notification area, where its icon brings it back."
  setting="closeBehaviour"
  value={preferences.closeBehaviour}
  onSelect={(closeBehaviour) => {
    onChange({ closeBehaviour });
  }}
/>
```

- [ ] **Step 8: Run everything**

Run: `cargo test -p osstat; npm run test --workspace @osstat/ui; npm run typecheck --workspace @osstat/ui`
Expected: all exit 0.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/window_state.rs src-tauri/src/lib.rs src-tauri/src/commands.rs ui/src/bindings ui/src/lib/ipc.ts ui/src/lib/preferences.ts ui/src/App.tsx ui/src/pages/Settings.tsx
git commit -m "feat(app): let closing the window hide it instead of quitting

The decision is held in Rust because CloseRequested must be answered
synchronously; there is no opportunity to ask the webview.

That also rules out a confirmation dialog, deliberately. It would have to
be drawn by the webview that is about to disappear, and if it ever failed
to draw, the close button would silently do nothing and osstat would be
dismissible only through its tray icon.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 10: The tray icon

**Files:**

- Create: `src-tauri/src/tray.rs`
- Modify: `src-tauri/Cargo.toml` (the `tray-icon` feature)
- Modify: `src-tauri/src/lib.rs` (module, `setup`)
- Modify: `src-tauri/src/sampler.rs` (tooltip refresh in `run`)

**Interfaces:**

- Consumes: `Activity` (Task 8), `CloseSetting` (Task 9).
- Produces:
  - `pub const TRAY_ID: &str`
  - `pub fn tooltip(sample: Option<&MetricsSample>, total_memory: u64) -> String`
  - `pub fn create<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()>`
  - `pub fn set_tooltip<R: Runtime>(app: &AppHandle<R>, text: &str)`
  - `pub fn show_main_window<R: Runtime>(app: &AppHandle<R>)`

- [ ] **Step 1: Enable the feature**

In `src-tauri/Cargo.toml`, change the `tauri` dependency to:

```toml
tauri = { version = "2.11.5", features = ["tray-icon"] }
```

- [ ] **Step 2: Write the failing tooltip test**

Create `src-tauri/src/tray.rs` containing only:

```rust
//! The notification-area icon.

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// A sample with only the two fields the tooltip reads set.
    ///
    /// Written out rather than using `Default`: `MetricsSample` does not derive
    /// it, and adding the derive purely for a test would put a meaningless
    /// all-zero sample into the public API.
    fn sample(cpu_total: f32, memory_used: u64) -> MetricsSample {
        MetricsSample {
            at_ms: 1_700_000_000_000,
            cpu_total,
            cpu_per_core: Vec::new(),
            memory_used,
            memory_available: 0,
            swap_used: 0,
            disks: Vec::new(),
            interfaces: Vec::new(),
            gpus: Vec::new(),
        }
    }

    #[test]
    fn says_it_is_starting_before_the_first_sample() {
        let text = tooltip(None, 64 * 1024 * 1024 * 1024);
        assert!(text.contains("osstat"));
        assert!(text.contains("starting"));
    }

    #[test]
    fn reports_cpu_and_memory_once_there_is_a_sample() {
        let text = tooltip(
            Some(&sample(23.4, 18 * 1024 * 1024 * 1024)),
            64 * 1024 * 1024 * 1024,
        );
        assert!(text.contains("23%"), "{text}");
        assert!(text.contains("GB"), "{text}");
    }

    #[test]
    fn stays_short_enough_for_a_tooltip() {
        // Windows truncates a tray tooltip at 128 characters.
        let text = tooltip(Some(&sample(100.0, 128 * 1024 * 1024 * 1024)), u64::MAX);
        assert!(text.len() < 128, "{} characters: {text}", text.len());
    }
}
```

- [ ] **Step 3: Run to verify it fails**

Add `pub mod tray;` to `src-tauri/src/lib.rs`, then run: `cargo test -p osstat tray`
Expected: FAIL — `tooltip` is not defined.

- [ ] **Step 4: Write the tray module**

Put this above the `tests` module in `src-tauri/src/tray.rs`:

```rust
//! The notification-area icon.
//!
//! The icon itself is static and the tooltip carries the data. Rendering load
//! into a ~16 px image would have to be done for three platforms and has almost
//! no room to say anything legible; a tooltip has room for the two figures that
//! answer "is this machine busy?".

use osstat_core::MetricsSample;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};

/// Identifies the tray icon, so the sampler thread can find it again.
pub const TRAY_ID: &str = "osstat-tray";

/// Menu item id for showing the window.
const SHOW_ID: &str = "tray-show";
/// Menu item id for quitting.
const QUIT_ID: &str = "tray-quit";

/// Renders the tooltip text.
///
/// @param sample The most recent sample, or `None` before the first tick.
/// @param total_memory Installed memory, for the denominator.
#[must_use]
pub fn tooltip(sample: Option<&MetricsSample>, total_memory: u64) -> String {
    let Some(sample) = sample else {
        return "osstat — starting…".to_owned();
    };

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a rounded percentage is 0..=100 by construction"
    )]
    let cpu = sample.cpu_total.round().clamp(0.0, 100.0) as u8;

    format!(
        "osstat — CPU {cpu}%  ·  Memory {} / {}",
        gibibytes(sample.memory_used),
        gibibytes(total_memory)
    )
}

/// Formats bytes as gibibytes to one decimal place.
fn gibibytes(bytes: u64) -> String {
    #[allow(
        clippy::cast_precision_loss,
        reason = "display precision; the value is rounded to one decimal anyway"
    )]
    let value = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    format!("{value:.1} GB")
}

/// Shows, unminimises and focuses the main window.
pub fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}

/// Creates the tray icon and its menu.
///
/// # Errors
///
/// Returns an error if the menu or the icon cannot be created. Callers are
/// expected to log and carry on: an app with no tray icon still works.
pub fn create<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, SHOW_ID, "Show osstat", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT_ID, "Quit osstat", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&show, &PredefinedMenuItem::separator(app)?, &quit])?;

    let icon = app.default_window_icon().cloned().ok_or_else(|| {
        tauri::Error::AssetNotFound("no default window icon to use for the tray".to_owned())
    })?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip(tooltip(None, 0))
        .menu(&menu)
        // Left-click shows the window: the Windows convention, and the first
        // thing anyone tries.
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if matches!(event, TrayIconEvent::Click { .. }) {
                show_main_window(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            SHOW_ID => show_main_window(app),
            QUIT_ID => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}

/// Replaces the tray tooltip, ignoring a failure.
///
/// A tooltip that could not be updated is not worth interrupting a tick for.
pub fn set_tooltip<R: Runtime>(app: &AppHandle<R>, text: &str) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(text));
    }
}
```

Add `use osstat_core::MetricsSample;` to the test module's `use super::*;` scope — it is already imported at module level.

- [ ] **Step 5: Refresh the tooltip from the sampler**

In `src-tauri/src/sampler.rs`, add `use crate::tray;` to the imports. `run` needs installed memory, so change its signature to take it and pass `description.total_memory` from `Sampler::start`:

```rust
fn run(
    app: &AppHandle,
    shared: &Arc<Shared>,
    mut source: SysinfoSource,
    mut gpus: HardwareProbe,
    total_memory: u64,
) {
```

In `Sampler::start`, change the spawn closure to `move || run(&app, &worker, source, HardwareProbe::new(system_memory), system_memory)`.

At the end of the loop body, after the history push, add:

```rust
        tray::set_tooltip(app, &tray::tooltip(Some(&sample), total_memory));
```

- [ ] **Step 6: Create the tray in `setup`**

In `src-tauri/src/lib.rs`, inside `setup`, after `app.manage(sampler);`:

```rust
            // A tray that could not be created is logged and moved past. An app
            // without a tray icon still works; an app that refuses to start
            // does not.
            if let Err(error) = tray::create(app.handle()) {
                eprintln!("osstat: could not create the tray icon: {error}");
            }
```

The start-at-sign-in checkbox is deliberately absent here. It belongs to the plugin Task 11 adds, and a checkbox that could not act on anything is exactly what §4.5 of the spec argues against.

- [ ] **Step 7: Run and check**

Run: `cargo test -p osstat; cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: both exit 0.

Then `npm run dev` and check: the tray icon appears; hovering shows live CPU and memory; left-click shows the window; Quit exits; closing the window hides it and the tooltip keeps updating.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/tray.rs src-tauri/src/lib.rs src-tauri/src/sampler.rs
git commit -m "feat(app): add a notification-area icon

Static icon, live tooltip. Rendering load into a ~16px image would have
to be done for three platforms and has almost no room to say anything
legible; a tooltip has room for the two figures that answer whether the
machine is busy.

A tray that cannot be created is logged and moved past. An app without a
tray icon still works; an app that refuses to start does not.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 11: Start at sign-in, and never twice

**Files:**

- Modify: `src-tauri/Cargo.toml` (two plugins)
- Modify: `src-tauri/src/lib.rs` (plugin registration, `--hidden`)
- Modify: `src-tauri/tauri.conf.json` (`"visible": false`)
- Modify: `src-tauri/capabilities/default.json` (autostart permissions)
- Modify: `package.json` / `ui/package.json` (the autostart JS binding)

**Interfaces:**

- Consumes: `tray::show_main_window` (Task 10).
- Produces: `pub fn starts_hidden(args: &[String]) -> bool` in `lib.rs`.

- [ ] **Step 1: Write the failing test**

Add a `tests` module at the end of `src-tauri/src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn an_ordinary_launch_shows_the_window() {
        assert!(!starts_hidden(&args(&["osstat.exe"])));
    }

    #[test]
    fn the_sign_in_launch_does_not() {
        assert!(starts_hidden(&args(&["osstat.exe", "--hidden"])));
    }

    #[test]
    fn the_flag_is_recognised_wherever_it_appears() {
        assert!(starts_hidden(&args(&["osstat.exe", "--verbose", "--hidden"])));
    }

    #[test]
    fn a_flag_that_merely_starts_the_same_way_is_not_it() {
        assert!(!starts_hidden(&args(&["osstat.exe", "--hidden-extras"])));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p osstat starts_hidden`
Expected: FAIL — `starts_hidden` is not defined.

- [ ] **Step 3: Add the dependencies**

In `src-tauri/Cargo.toml`, under `[dependencies]`:

```toml
tauri-plugin-autostart = "2"
tauri-plugin-single-instance = "2"
```

Then in the repository root:

```bash
npm install --workspace @osstat/ui @tauri-apps/plugin-autostart
```

- [ ] **Step 4: Register the plugins**

In `src-tauri/src/lib.rs`, add the imports:

```rust
use tauri_plugin_autostart::MacosLauncher;
```

Add this function above `run`:

```rust
/// The flag the sign-in entry adds, so a login launch opens no window.
const HIDDEN_FLAG: &str = "--hidden";

/// Whether this process was started by the sign-in entry.
///
/// @param args The process arguments, including the program name.
#[must_use]
pub fn starts_hidden(args: &[String]) -> bool {
    args.iter().any(|arg| arg == HIDDEN_FLAG)
}
```

Change the builder chain to begin:

```rust
    tauri::Builder::default()
        // Registered first, as this plugin requires. Once osstat starts at
        // sign-in, clicking the desktop shortcut would otherwise launch a rival
        // copy: two tray icons, two samplers, two windows with one title.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main_window(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![HIDDEN_FLAG]),
        ))
        .setup(|app| {
```

And inside `setup`, after the tray is created:

```rust
            if !starts_hidden(&std::env::args().collect::<Vec<_>>())
                && let Some(window) = app.get_webview_window("main")
            {
                let _ = window.show();
            }
```

Add `use tauri::Manager;` if it is not already imported (it is, at `lib.rs:16`).

- [ ] **Step 5: Add the tray checkbox, now that there is a plugin behind it**

In `src-tauri/src/tray.rs`, add the import `use tauri_plugin_autostart::ManagerExt;`, restore the id constant:

```rust
/// Menu item id for the start-at-sign-in checkbox.
const AUTOSTART_ID: &str = "tray-autostart";
```

change `create` to build it, reading the current state from the OS rather than from anything stored:

```rust
    let show = MenuItem::with_id(app, SHOW_ID, "Show osstat", true, None::<&str>)?;
    let autostart = CheckMenuItem::with_id(
        app,
        AUTOSTART_ID,
        "Start osstat when I sign in",
        true,
        // Read from the operating system. There is no stored copy to consult,
        // deliberately: a mirror could disagree with reality the moment someone
        // removed the entry through Task Manager's Startup tab.
        app.autolaunch().is_enabled().unwrap_or(false),
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, QUIT_ID, "Quit osstat", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &show,
            &PredefinedMenuItem::separator(app)?,
            &autostart,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
```

add `CheckMenuItem` back to the `tauri::menu` import, and extend the menu handler:

```rust
        .on_menu_event(|app, event| match event.id().as_ref() {
            SHOW_ID => show_main_window(app),
            AUTOSTART_ID => {
                let manager = app.autolaunch();
                // Toggle against what the OS currently says, not against what
                // the checkbox is showing: another window, or Task Manager, may
                // have changed it since this menu was built.
                let _ = if manager.is_enabled().unwrap_or(false) {
                    manager.disable()
                } else {
                    manager.enable()
                };
            }
            QUIT_ID => app.exit(0),
            _ => {}
        })
```

- [ ] **Step 6: Start the window hidden**

In `src-tauri/tauri.conf.json`, add `"visible": false` to the `main` window object, after `"center": true`.

This also removes an existing flash: the window no longer paints before React mounts.

- [ ] **Step 7: Grant the plugin's permissions**

Replace `src-tauri/capabilities/default.json` with:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Baseline capability for the main window. Deliberately minimal: core defaults, plus reading and writing this app's own sign-in entry. Privileged capabilities are added per feature, never pre-emptively, so the webview's reach stays auditable (ADR-001, ADR-006).",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "autostart:allow-is-enabled",
    "autostart:allow-enable",
    "autostart:allow-disable"
  ]
}
```

- [ ] **Step 8: Run and check**

Run: `cargo test -p osstat; cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: both exit 0.

Then `npm run dev` and check: the window still opens; the tray menu's sign-in checkbox reflects Task Manager's Startup tab and changes it; launching a second copy of the built binary focuses the first rather than starting another.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/lib.rs src-tauri/src/tray.rs src-tauri/tauri.conf.json src-tauri/capabilities/default.json package.json package-lock.json ui/package.json
git commit -m "feat(app): start at sign-in, and only ever once

Single-instance is registered first, as the plugin requires, and is not
optional: once osstat starts at sign-in, clicking the desktop shortcut
would otherwise launch a rival copy -- two tray icons, two samplers, two
windows with one title. A second launch now focuses the first.

The sign-in entry carries --hidden and the window starts invisible, so a
login launch paints nothing. That also removes an existing flash of
unstyled window before React mounts on an ordinary launch.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 12: The startup toggle, and telling the user where the window went

**Files:**

- Modify: `ui/src/pages/Settings.tsx`
- Modify: `ui/src/lib/preferences.ts` (the seen-notice flag)
- Modify: `ui/src/pages/Overview.tsx` (the banner)
- Test: `ui/src/pages/Settings.test.tsx`, `ui/src/pages/Overview.test.tsx`

**Interfaces:**

- Consumes: `@tauri-apps/plugin-autostart` (`isEnabled`, `enable`, `disable`); `Preferences` from Task 3 and Task 9.
- Produces: `Preferences.hasSeenTrayNotice: boolean`, default `false`.

- [ ] **Step 1: Write the failing tests**

Append to `ui/src/pages/Settings.test.tsx`:

```tsx
vi.mock('@tauri-apps/plugin-autostart', () => ({
  isEnabled: vi.fn(async () => false),
  enable: vi.fn(async () => {}),
  disable: vi.fn(async () => {}),
}));

describe('Settings › Start at sign-in', () => {
  it('reflects what the operating system actually has registered', async () => {
    const { isEnabled } = await import('@tauri-apps/plugin-autostart');
    vi.mocked(isEnabled).mockResolvedValueOnce(true);

    render(<Settings preferences={prefs()} onChange={vi.fn()} />);

    expect(await screen.findByRole('switch', { name: /sign in/i })).toBeChecked();
  });

  it('writes to the operating system, not to a stored copy', async () => {
    const { enable } = await import('@tauri-apps/plugin-autostart');
    render(<Settings preferences={prefs()} onChange={vi.fn()} />);

    fireEvent.click(await screen.findByRole('switch', { name: /sign in/i }));

    expect(enable).toHaveBeenCalled();
  });

  it('says so rather than lying when the entry cannot be read', async () => {
    const { isEnabled } = await import('@tauri-apps/plugin-autostart');
    vi.mocked(isEnabled).mockRejectedValueOnce(new Error('registry unavailable'));

    render(<Settings preferences={prefs()} onChange={vi.fn()} />);

    expect(await screen.findByRole('alert')).toHaveTextContent(/registry unavailable/i);
  });
});
```

Append to `ui/src/pages/Overview.test.tsx`:

```tsx
describe('the tray notice', () => {
  it('explains where the window went', () => {
    render(<Overview {...overviewProps({ showTrayNotice: true })} />);
    expect(screen.getByRole('status', { name: /notification area/i })).toBeInTheDocument();
  });

  it('stays away once it has been seen', () => {
    render(<Overview {...overviewProps({ showTrayNotice: false })} />);
    expect(screen.queryByRole('status', { name: /notification area/i })).not.toBeInTheDocument();
  });

  it('records that it has been seen when dismissed', () => {
    const onTrayNoticeSeen = vi.fn();
    render(<Overview {...overviewProps({ showTrayNotice: true, onTrayNoticeSeen })} />);

    fireEvent.click(screen.getByRole('button', { name: /dismiss/i }));

    expect(onTrayNoticeSeen).toHaveBeenCalled();
  });
});
```

Add an `overviewProps` helper to that file if one is not already there, built from the fixtures the existing tests use:

```tsx
function overviewProps(overrides: Partial<OverviewProps> = {}): OverviewProps {
  return {
    system: SYSTEM_FIXTURE,
    samples: [],
    latest: null,
    gpus: null,
    layout: 'onePage',
    panels: [],
    onPanelsChange: vi.fn(),
    showTrayNotice: false,
    onTrayNoticeSeen: vi.fn(),
    ...overrides,
  };
}
```

`SYSTEM_FIXTURE` is whatever `SystemDescription` the existing tests in that file already build; reuse it rather than writing a second one. Note the `showTrayNotice` boolean is what `Overview` takes — `hasSeenTrayNotice` is the stored preference, and `App` converts one into the other.

- [ ] **Step 2: Run to verify they fail**

Run: `npm run test --workspace @osstat/ui -- Settings Overview`
Expected: FAIL — no switch, no notice.

- [ ] **Step 3: Add the preference**

In `ui/src/lib/preferences.ts`, add `hasSeenTrayNotice: boolean;` to `Preferences`, `hasSeenTrayNotice: false,` to `DEFAULT_PREFERENCES`, and to `coercePreferences`:

```ts
    // A plain boolean rather than a CHOICES entry: it is a record of something
    // that happened, not a setting anyone picks.
    hasSeenTrayNotice: candidate.hasSeenTrayNotice === true,
```

- [ ] **Step 4: Add the startup toggle**

In `ui/src/pages/Settings.tsx`, add:

```tsx
import { useEffect, useState } from 'react';
import { disable, enable, isEnabled } from '@tauri-apps/plugin-autostart';
```

and this component:

```tsx
/**
 * The start-at-sign-in switch.
 *
 * Reads and writes the operating system directly. There is deliberately no
 * stored copy: a mirror could disagree with reality the moment someone removed
 * the entry through Task Manager's Startup tab, and a switch that lies about
 * what the machine will do at sign-in is worse than no switch.
 */
function StartAtSignIn(): React.JSX.Element {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    isEnabled().then(setEnabled, (error: unknown) => {
      setProblem(error instanceof Error ? error.message : String(error));
    });
  }, []);

  const toggle = (): void => {
    const next = enabled !== true;
    const applied = next ? enable() : disable();

    applied.then(
      () => {
        setEnabled(next);
        setProblem(null);
      },
      (error: unknown) => {
        setProblem(error instanceof Error ? error.message : String(error));
      }
    );
  };

  return (
    <div className="flex flex-col gap-2 border-b border-edge px-4 py-3 last:border-b-0">
      <div className="flex items-start justify-between gap-4">
        <div>
          <p className="text-sm">Start osstat when I sign in</p>
          <p className="text-xs text-neutral-500">
            Starts in the notification area, with no window.
          </p>
        </div>

        <button
          type="button"
          role="switch"
          aria-checked={enabled === true}
          aria-label="Start osstat when I sign in"
          disabled={enabled === null && problem === null}
          onClick={toggle}
          className={`h-5 w-9 shrink-0 rounded-full border transition-colors ${
            enabled === true ? 'border-accent bg-accent/40' : 'border-edge bg-white/[0.04]'
          }`}
        >
          <span
            aria-hidden="true"
            className={`block h-3.5 w-3.5 rounded-full bg-neutral-200 transition-transform ${
              enabled === true ? 'translate-x-4.5' : 'translate-x-0.5'
            }`}
          />
        </button>
      </div>

      {problem !== null && (
        <p role="alert" className="text-xs text-red-400">
          Could not read or change the sign-in entry: {problem}
        </p>
      )}
    </div>
  );
}
```

Add `<StartAtSignIn />` to the settings card, above the `When I close the window` choice.

- [ ] **Step 5: Add the banner**

In `ui/src/pages/Overview.tsx`, add to `OverviewProps`:

```tsx
  /** Whether to explain that closing the window did not quit. */
  showTrayNotice: boolean;
  /** Records that the explanation has been read. */
  onTrayNoticeSeen: () => void;
```

and render it at the top of the returned fragment, above the header:

```tsx
{
  props.showTrayNotice && (
    <div
      role="status"
      aria-label="osstat keeps running in the notification area"
      className="flex items-start gap-3 rounded-lg border border-edge bg-surface-raised px-4 py-2.5 text-xs text-neutral-300"
    >
      <span className="flex-1">
        osstat kept running in the notification area when you closed the window. You can change that
        in Settings.
      </span>
      <button
        type="button"
        aria-label="Dismiss"
        onClick={props.onTrayNoticeSeen}
        className="text-neutral-500 hover:text-neutral-200"
      >
        ✕
      </button>
    </div>
  );
}
```

In `ui/src/App.tsx`, pass them:

```tsx
          showTrayNotice={!preferences.hasSeenTrayNotice && preferences.closeBehaviour === 'hide'}
          onTrayNoticeSeen={() => {
            onPreferenceChange({ hasSeenTrayNotice: true });
          }}
```

- [ ] **Step 6: Run everything**

Run: `npm run test --workspace @osstat/ui; npm run typecheck --workspace @osstat/ui; npm run lint --workspace @osstat/ui; cargo test -p osstat`
Expected: all exit 0.

- [ ] **Step 7: Verify Phase B by hand**

Run `npm run build` and launch the built binary from `src-tauri/target/release/`.

1. Settings › Start osstat when I sign in → on. Check Task Manager's Startup tab lists osstat.
2. Sign out and back in. osstat appears in the tray with no window.
3. Hover the tray icon — CPU and memory update roughly every 5 s.
4. Click the icon — the window opens, and the banner explains the tray.
5. Dismiss the banner, close and reopen — it does not return.
6. Settings › When I close the window → Quit. Close. The app exits.
7. Turn the sign-in toggle off. Task Manager's Startup tab no longer lists it.

- [ ] **Step 8: Commit**

```bash
git add ui/src/pages/Settings.tsx ui/src/pages/Settings.test.tsx ui/src/pages/Overview.tsx ui/src/pages/Overview.test.tsx ui/src/lib/preferences.ts ui/src/App.tsx
git commit -m "feat(ui): add the start-at-sign-in switch and the tray notice

The switch reads and writes the operating system directly, with no
stored copy. A mirror could disagree with reality the moment someone
removed the entry through Task Manager's Startup tab, and a switch that
lies about what the machine will do at sign-in is worse than no switch.
When the entry cannot be read, it says so rather than showing a
plausible default.

The one-time banner explains where the window went. It appears when
there is a window to show it in, which a close-time dialog could not
guarantee.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Closing checks

- [ ] Run `just ci`.

**Known unrelated failure:** `npm run format:check` reports `.claude-flow/*.json`, untracked local tooling output that commit `dc1c565` deliberately removed from `.gitignore`. Everything else must pass. Adding `.claude-flow/` to `.prettierignore` clears it, but that is the repository owner's call, not this plan's.

- [ ] Confirm `cargo bench --workspace` still meets the M1 gate — Task 8 changed the sampler loop, and the process read is now conditional.
- [ ] Confirm the release binary has not grown unreasonably: two plugins and the tray were added to a 4.57 MB baseline against a ~20 MB budget.
