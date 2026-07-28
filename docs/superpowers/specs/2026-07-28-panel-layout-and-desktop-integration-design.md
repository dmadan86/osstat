# Arrangeable Overview panels, and osstat as a resident desktop app

**Date:** 2026-07-28
**Status:** Approved, not yet implemented.
**Supersedes:** nothing. **Depends on:** ADR-001, ADR-002, ADR-007.
**Builds on:** [2026-07-28-interactive-dashboard-design.md](2026-07-28-interactive-dashboard-design.md)

## Context

The interactive dashboard shipped a fixed Overview: five sections — CPU, Memory,
Disks, Network, GPU — always in the same order, always full width, each one
collapsible and nothing more. Which of those matters most depends entirely on
why someone opened the app, and the app currently has no way to be told.

Separately, osstat is still a program you launch, look at, and close. A machine
monitor is more useful as something that is simply _there_: running from sign-in,
reachable from the notification area, out of the way until wanted.

This spec covers both: arranging the Overview, and living on the desktop.

## Goals

1. The user decides the order, size and presence of the Overview panels, and the
   choice survives a restart.
2. Every arrangement action is reachable without a mouse.
3. osstat can start at sign-in and stay resident in the notification area.
4. Running hidden costs close to nothing, and the app says nothing it has not
   measured.

## Non-goals

- **Arranging any page other than Overview.** Processes is a virtualised table;
  "make this a third wide" has no meaning there.
- **Free placement.** Panels flow; they do not hold absolute grid coordinates,
  and a layout can never contain a hole the user has to tidy up.
- **Drag-resize.** Size is chosen from a menu. See §2.2.
- **Multiple saved layouts or profiles.** One arrangement, plus a reset.
- **A live tray icon.** The icon is static; the tooltip carries the data. See
  §4.3.

---

## 1. Decisions taken during design

Recorded because each closed off an option that will look attractive again later.

| Decision                                        | Rejected alternative                     | Why                                                                                                                      |
| ----------------------------------------------- | ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| Size from a menu, order by drag                 | Drag-resize handles; free grid placement | Panels hold charts with hover crosshairs. Edge-dragging and chart-reading would be the same gesture.                     |
| Hide from the panel menu, restore from Settings | Restore control on Overview              | Keeps Overview free of an affordance that is empty most of the time.                                                     |
| Autostart state lives only in the OS            | Mirrored in `localStorage`               | A mirror can disagree with reality the moment someone removes the entry via Task Manager's Startup tab.                  |
| Close hides, with no confirmation dialog        | Confirm-on-close                         | The dialog would be drawn by the webview that is about to vanish. If it ever failed to draw, the window could not close. |
| Hidden means a slow metrics-only tick           | Full sampling; or no sampling at all     | The process read is 23.8 ms of a ~24 ms tick and nothing in a tooltip needs it.                                          |

---

## 2. Overview panel layout

### 2.1 The stored shape

```ts
/** Twelfths of the row a panel occupies. */
export type PanelSpan = 4 | 6 | 8 | 12; // third, half, two-thirds, full

/** How much vertical room a panel's body gets. */
export type PanelHeight = 'short' | 'normal' | 'tall';

export interface PanelLayout {
  /** Matches `SectionSpec.id`. */
  id: string;
  span: PanelSpan;
  height: PanelHeight;
  hidden: boolean;
}
```

Order is the array's order; there is no separate index field to keep consistent.

This joins `Preferences` as `overviewPanels: PanelLayout[]` in the existing
`osstat.preferences.v1` record. **No key bump and no migration:**
`coercePreferences` already falls back field by field, so a record written by the
current build simply has no panel list and receives the default.

`PANEL_HEIGHTS = { short: 96, normal: 140, tall: 220 }` gives the pixel height
for a panel's chart. A panel with no chart applies the same value as a
`min-height` on its body, so a row of half-width panels aligns instead of
ragging.

**Below 900 px of content width, every panel spans the full row** regardless of
its stored span. The window's `minWidth` is 900, so this is reachable only on a
narrow display or with the sidebar expanded on a small screen; a third-width
chart at that size is unreadable, and honouring the preference there would serve
nobody. The stored span is not modified — it applies again when there is room.

**Hiding every panel is permitted.** Overview then shows only its header and a
line pointing at Settings > Panels. Refusing the last hide would mean explaining
an arbitrary rule; an empty page that says how to undo itself does not.

### 2.2 Reconciliation

The stored list and the code's section list _will_ drift: a release adds a
section, or removes one, and a saved layout predates it. The stored list is
therefore never treated as the authority on what exists.

`reconcileLayout(stored, sections)` runs on every render and:

- keeps sections named in `stored`, **in `stored`'s order**;
- **appends** sections `stored` does not mention, in canonical order — a newly
  added panel appears, rather than vanishing because an old layout never knew
  about it;
- **drops** stored ids with no matching section, and any repeat of an id already
  seen — first occurrence wins, so a hand-edited or double-written record cannot
  make a panel render twice;
- clamps any span or height outside the permitted set.

It is a pure function over two arrays and carries the bulk of the tests.

### 2.3 Interaction

**Reorder — drag.** A grip (`⠿`) at the left of each panel header. Pointer-down
captures the pointer. During a drag, panel bodies get `pointer-events: none` so
ECharts cannot swallow the gesture; the dragged panel dims and a drop indicator
marks the target slot. Drop position comes from hit-testing panel midpoints.
Escape cancels and restores the original order.

Implemented with pointer events directly rather than a drag library. The problem
is reordering five cards in a flow container, keyboard support is already covered
by the menu below, and this follows `Chart.tsx`, which was hand-written for the
same reason.

**Everything else — the `⋮` menu** on each panel header:

| Group  | Items                            |
| ------ | -------------------------------- |
| Width  | Third · Half · Two-thirds · Full |
| Height | Short · Normal · Tall            |
| Order  | Move up · Move down              |
|        | Hide this panel                  |

Because _Move up_ and _Move down_ exist, dragging is a pointer-only enhancement.
No keyboard user is ever asked to perform a drag, which is where accessible
drag-and-drop usually fails.

**When `pageLayout` is `subTabs`**, the menu shows only _Move up_, _Move down_
and _Hide_. Width and height mean nothing when one section fills the pane, and
offering a control that does nothing is worse than omitting it. Order still
matters — it is the tab order.

### 2.4 A required change to `Section.tsx`

Today the whole section header is one `<button>` that toggles collapse. A grip
and a menu button cannot nest inside it; nested buttons are invalid HTML and
behave inconsistently across browsers.

The header becomes a flex container with three siblings: the grip, a collapse
button wrapping the title and summary, and the menu button. Appearance is
unchanged; the semantics become valid.

### 2.5 Settings

A **Panels** section listing every Overview panel with a visible/hidden checkbox
— the way back for anything hidden from the panel menu — and a **Reset Overview
layout** button that clears order, sizes and hidden flags together. The reset is
what makes experimenting with the layout safe.

---

## 3. What runs while osstat is hidden

### 3.1 Three activity states

`Control.paused: bool` cannot express "hidden but still feeding a tooltip", so it
becomes:

| State        | Interval       | Reads                     | Emits to the webview             | Tray tooltip |
| ------------ | -------------- | ------------------------- | -------------------------------- | ------------ |
| `Foreground` | user's setting | metrics **and** processes | `metrics:tick`, `processes:tick` | refreshed    |
| `Background` | 5 s, fixed     | metrics only              | nothing                          | refreshed    |
| `Paused`     | —              | nothing                   | nothing                          | unchanged    |

The background interval is deliberately not configurable. It exists to keep a
tooltip current, and a setting for it would be a knob whose only effect is how
much battery osstat burns while nobody is looking.

`Background` skips the process table because that is where the cost is, and emits
no events because delivering IPC payloads to a hidden webview is work nobody can
observe. History keeps filling, so reopening the window shows the interval rather
than a hole.

`activity_for(visible, minimized, user_paused) -> Activity` is a pure function and
is where this is tested; the `WindowEvent` plumbing around it is not testable
in-process. `Paused` — the user's explicit Settings choice — always wins.
Otherwise a hidden or minimised window means `Background`.

This widens the existing rule rather than replacing it: sampling still never
stops on mere loss of focus, for the reason given in `sampler.rs`.

### 3.2 A latent charting flaw this makes routine

`options.ts:52` builds the x-axis as `type: 'category'` with one slot per sample.
Slots are drawn at equal widths regardless of when the samples were taken, so a
stretch collected every 5 s renders exactly as wide as a stretch collected every
2 s — the chart would misstate time.

This is already latent: minimising leaves a gap in history that a category axis
draws as continuous. Background sampling turns a rare case into a routine one.

**The four time axes therefore move to `type: 'time'`**, plotting `[at, value]`
pairs. `TimePoint` already carries `at` in milliseconds, so the change is small,
and it makes both the new irregular spacing and the pre-existing gaps render
truthfully.

---

## 4. Desktop integration

### 4.1 Plugins

| Plugin                         | Why                                                    |
| ------------------------------ | ------------------------------------------------------ |
| `tauri-plugin-single-instance` | **Required.** Registered first, as the plugin demands. |
| `tauri-plugin-autostart`       | One API over three quite different per-OS mechanisms.  |

Tray support is built into Tauri 2 and needs only the `tray-icon` feature.

Single-instance is not optional once osstat starts at sign-in: clicking the
desktop shortcut would otherwise launch a rival copy — two tray icons, two
samplers, two windows with the same title. Instead, a second launch shows,
unminimises and focuses the running instance, which is also the right response to
clicking the shortcut while osstat sits in the tray.

Both plugins need matching entries in `src-tauri/capabilities/default.json`.

### 4.2 Tray

Created in `setup` with a stable id, so the sampler thread can reach it through
`app.tray_by_id`.

```
Show osstat
─────────────────
Start osstat when I sign in   ☑
─────────────────
Quit osstat
```

Left-clicking the icon shows and focuses the window — the Windows convention and
the first thing people try.

### 4.3 Tooltip

Refreshed each tick, in every state that samples:

```
osstat — CPU 23%  ·  Memory 18.4 / 63.7 GB
```

Before the first sample it reads `osstat — starting…`. The icon itself stays
static: rendering load into a ~16 px image would have to be done for three
platforms and has almost no room to say anything legible.

### 4.4 Closing the window

`CloseRequested` consults one piece of state held in Rust — `CloseBehaviour`,
either `Hide` (the default) or `Quit` — and either calls `prevent_close()` then
`hide()`, or lets the close proceed and exits.

It is held in Rust, not read from the webview at close time, because the decision
has to be made synchronously inside the event handler. The Settings control
writes it through a `set_close_behaviour` command and mirrors it into
`localStorage`; the front-end replays that value to Rust on startup, the same way
it already replays the refresh interval. If the two ever disagree — a crash
between the two writes — Rust's value governs this window's behaviour and the
next startup reconciles it.

There is deliberately no confirmation dialog. Such a dialog would have to be
rendered by the webview that is about to disappear; if it ever failed to render,
the close button would silently do nothing and osstat would be dismissible only
through the tray. Reading a value Rust already holds cannot get stuck that way.

Two things make the behaviour non-surprising instead:

- **Settings: "When I close the window — Hide to the notification area / Quit
  osstat."** Default hide. Discoverable, and changeable.
- **A one-time banner** on Overview the next time the window is shown: _"osstat
  kept running in the notification area. You can change this in Settings."_
  Dismissible, shown once, recorded in preferences. It appears when there is a
  window to show it in, which a close-time dialog cannot guarantee.

### 4.5 Starting at sign-in

The Settings toggle and the tray checkbox both read and write the plugin
directly. The OS entry is the single record, so the two controls cannot disagree
with each other or drift from what Task Manager's Startup tab shows.

The registered command line carries `--hidden`. The `main` window becomes
`"visible": false` in `tauri.conf.json` and `setup` shows it unless that flag is
present — so an ordinary launch does not flash an unstyled window before React
mounts, and a sign-in launch never paints one at all.

---

## 5. Failure behaviour

Each piece fails alone.

- **Tray creation fails** — log it and open the window normally. An app without a
  tray icon still works; an app that refuses to start does not.
- **Autostart cannot be read or written** — the toggle shows the true state with
  the error beside it, never a checkbox that lies about what the OS will do.
- **A tick fails** — skipped, as today.
- **Stored layout is unreadable** — `coercePreferences` already falls back per
  field; §2.2 then repairs anything structurally wrong. Reading preferences must
  never be able to white-screen the app.

---

## 6. Testing

**Rust**

- `activity_for` across the full matrix of visible × minimised × user-paused.
- The interval each activity state selects.
- Tooltip formatting, including before any sample exists.
- `--hidden` argument parsing.
- `CloseBehaviour` defaulting to `Hide`, and surviving a `set_close_behaviour`
  round trip.

Tray and plugin registration cannot be stood up in a unit test; they are covered
by keeping every decision in the pure helpers above.

**TypeScript**

- `reconcileLayout`: a section added, removed, reordered, duplicated, and an
  illegal span or height.
- `reorder(list, from, to)`: both ends, and the no-op.
- `coercePanelLayout` against garbage input.
- The panel menu's actions, the Settings panel checkboxes and the reset.
- The one-time banner appearing once and not again.
- The time-axis change against irregularly spaced points.
- One jsdom test driving synthetic pointer events through a drag — enough to
  catch a wiring regression, and not represented as proof that a real drag works.

## 7. Out of scope, deliberately

Process termination remains unspecified and unbuilt; it still needs its own spec
covering SIGTERM/SIGKILL escalation, critical-process guards and ADR-006
elevation.
