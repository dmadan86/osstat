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
