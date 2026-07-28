/**
 * The pages the app has, defined once.
 *
 * All three navigation styles read this array, so adding a capability is one
 * entry here rather than three edits in three components. It is also what keeps
 * the styles honestly interchangeable: they cannot drift apart if they cannot
 * disagree about what exists.
 *
 * No router. Six pages in a desktop app with no URL bar and no back button does
 * not justify the dependency, and the thing a router is actually for — URLs —
 * is not available to us.
 */

/** Every page in the app. */
export type Route = 'overview' | 'processes' | 'ports' | 'cleaner' | 'llm' | 'settings';

/** One entry in the navigation. */
export interface NavItem {
  /** The page this entry leads to. */
  route: Route;
  /** What it is called. */
  label: string;
  /** A glyph for the icon rail, which has no room for words. */
  icon: string;
  /**
   * The milestone that builds this page, for the ones that do not exist yet.
   *
   * Present means unbuilt. The navigation dims these and the page shows what is
   * coming rather than pretending to be empty.
   */
  milestone?: string;
}

/** The capability pages, in the order they appear. */
export const NAV_ITEMS: readonly NavItem[] = [
  { route: 'overview', label: 'Overview', icon: '◧' },
  { route: 'processes', label: 'Processes', icon: '▤' },
  { route: 'ports', label: 'Ports', icon: '⇄', milestone: 'M2' },
  { route: 'cleaner', label: 'Cleaner', icon: '✦', milestone: 'M3' },
  { route: 'llm', label: 'LLM advisor', icon: '◇', milestone: 'M4' },
];

/** Settings, kept apart from the capabilities in every navigation style. */
export const SETTINGS_ITEM: NavItem = {
  route: 'settings',
  label: 'Settings',
  icon: '⚙',
};

/** What each unbuilt page will do, shown on its placeholder. */
export const PLANNED: Record<string, { title: string; summary: string }> = {
  ports: {
    title: 'Port inspector',
    summary:
      'Which process holds which port, and closing it on demand — joined to the process table you can already see.',
  },
  cleaner: {
    title: 'System cleaner',
    summary:
      'Caches, logs and dev-tool junk, defined in readable TOML manifests. Nothing is ever deleted without a preview first.',
  },
  llm: {
    title: 'LLM runnability advisor',
    summary:
      'Which local models this machine can actually run, with the arithmetic shown rather than hidden. The hardware probe behind it already works — see the GPU section on Overview.',
  },
};
