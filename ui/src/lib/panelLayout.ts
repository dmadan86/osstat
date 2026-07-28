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
 * This counts hidden panels as positions, same as everything else in the
 * list: it is the honest primitive for "move within the whole array." Menus
 * that only offer moves among the *visible* panels want
 * {@link moveVisibleById} instead — otherwise a hidden panel sitting between
 * two visible ones silently absorbs the move.
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
 * Reorders the visible panels, leaving hidden ones where they sit.
 *
 * Hidden panels keep their slots in the stored array, so an arrangement made
 * while something was hidden still means what the user intended when it comes
 * back. Only the visible slots are rewritten, in the order given.
 *
 * @param layout The full stored layout.
 * @param visibleIds The ids of the visible panels, in their new order.
 */
export function applyVisibleOrder(
  layout: readonly PanelLayout[],
  visibleIds: readonly string[]
): PanelLayout[] {
  const byId = new Map(layout.map((panel) => [panel.id, panel]));
  const next = [...layout];
  let cursor = 0;

  for (let index = 0; index < next.length; index += 1) {
    const panel = next[index];
    if (panel === undefined || panel.hidden) continue;

    const nextId = visibleIds[cursor];
    cursor += 1;
    if (nextId === undefined) continue;

    const replacement = byId.get(nextId);
    if (replacement !== undefined) next[index] = replacement;
  }

  return next;
}

/**
 * Moves a visible panel one place among the other visible panels.
 *
 * Distinct from {@link moveById}, which counts hidden panels as positions: a
 * hidden panel between two visible ones would otherwise absorb the move and
 * the click would look like it did nothing.
 *
 * @param layout The full stored layout.
 * @param id The panel to move.
 * @param delta `-1` for up, `1` for down.
 */
export function moveVisibleById(
  layout: readonly PanelLayout[],
  id: string,
  delta: number
): PanelLayout[] {
  const visibleIds = layout.filter((panel) => !panel.hidden).map((panel) => panel.id);

  const from = visibleIds.indexOf(id);
  if (from === -1) return [...layout];

  const to = from + delta;
  if (to < 0 || to >= visibleIds.length) return [...layout];

  return applyVisibleOrder(layout, reorder(visibleIds, from, to));
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
