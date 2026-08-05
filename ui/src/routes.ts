/**
 * The pages the app has, defined once.
 *
 * All three navigation styles read this array, so adding a capability is one
 * entry here rather than three edits in three components. It is also what keeps
 * the styles honestly interchangeable: they cannot drift apart if they cannot
 * disagree about what exists.
 *
 * Every entry here leads somewhere real. The navigation used to carry Cleaner
 * as a dimmed entry with an M3 badge, leading to a placeholder that described
 * what the page would eventually do — the idea being that hiding future work
 * understates the app. In practice it put a control in the navigation that
 * could not do the thing its label named, which is the one promise a
 * navigation makes. ROADMAP.md is where the plan belongs; when M3 builds the
 * cleaner, it arrives here as a page rather than as an advertisement for one.
 *
 * No router. Six pages in a desktop app with no URL bar and no back button does
 * not justify the dependency, and the thing a router is actually for — URLs —
 * is not available to us.
 */

import type { IconName } from './components/Icon';

/** Every page in the app. */
export type Route = 'overview' | 'processes' | 'ports' | 'llm' | 'chat' | 'settings';

/** One entry in the navigation. */
export interface NavItem {
  /** The page this entry leads to. */
  route: Route;
  /** What it is called. */
  label: string;
  /** A drawing for the icon rail, which has no room for words. */
  icon: IconName;
}

/** The capability pages, in the order they appear. */
export const NAV_ITEMS: readonly NavItem[] = [
  { route: 'overview', label: 'Overview', icon: 'grid' },
  { route: 'processes', label: 'Processes', icon: 'list' },
  { route: 'ports', label: 'Ports', icon: 'transfer' },
  { route: 'llm', label: 'LLM advisor', icon: 'cube' },
  { route: 'chat', label: 'Chat', icon: 'chat' },
];

/** Settings, kept apart from the capabilities in every navigation style. */
export const SETTINGS_ITEM: NavItem = {
  route: 'settings',
  label: 'Settings',
  icon: 'sliders',
};
