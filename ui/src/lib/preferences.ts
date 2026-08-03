/**
 * View preferences: how the app is laid out and how fast it samples.
 *
 * Two settings shape the layout, and they are deliberately **independent**
 * rather than three bundled presets. Where navigation lives and how a page is
 * sliced are separate questions, and pairing "icon rail" with "sub-tabs"
 * because they happened to be drawn together would be an arbitrary
 * restriction. Six combinations cost barely more than three.
 *
 * Persistence is `localStorage` behind this module's interface, which is the
 * shape a Rust-backed config store would expose later. A real config file is
 * the right end state and arrives when the cleaner needs actual settings; it is
 * not worth pulling in for four enums.
 */

import { useCallback, useEffect, useState } from 'react';

import type { CloseBehaviour } from '../bindings/CloseBehaviour';
import type { LogLevel } from '../bindings/LogLevel';
import { coercePanelLayout, type PanelLayout } from './panelLayout';
import { DEFAULT_THEME, THEMES, type Theme } from './theme';

/** Where the primary navigation lives. */
export type NavigationStyle = 'sidebar' | 'tabs' | 'rail';

/** How a page presents its sections. */
export type PageLayout = 'onePage' | 'subTabs';

/** Everything the user can choose. */
export interface Preferences {
  /** Which of the four dark themes the interface wears. */
  theme: Theme;
  /** Where the primary navigation lives. */
  navigation: NavigationStyle;
  /** Whether sections stack on one page or become sub-tabs. */
  pageLayout: PageLayout;
  /** Milliseconds between samples; `0` pauses sampling entirely. */
  refreshMs: number;
  /** Seconds of history the charts show. */
  historySeconds: number;
  /** What closing the window does. */
  closeBehaviour: CloseBehaviour;
  /** How much detail the log carries. */
  logLevel: LogLevel;
  /** Whether the one-time explanation of the tray has been dismissed. */
  hasSeenTrayNotice: boolean;
  /**
   * How the Overview panels are arranged.
   *
   * Empty means "never arranged"; the canonical section order is used, and
   * `reconcileLayout` fills this in from the sections that actually exist.
   */
  overviewPanels: PanelLayout[];
}

/** The allowed value of each setting, used for both the UI and validation. */
export const CHOICES = {
  // Listed here for the same validation every other setting gets, but rendered
  // by `Settings` itself rather than through `Choice`: a theme is picked by
  // looking at it, so each option carries a swatch that a row of text buttons
  // has nowhere to put.
  theme: THEMES,
  navigation: [
    { value: 'sidebar', label: 'Sidebar' },
    { value: 'tabs', label: 'Top tabs' },
    { value: 'rail', label: 'Icon rail' },
  ],
  pageLayout: [
    { value: 'onePage', label: 'One page' },
    { value: 'subTabs', label: 'Sub-tabs' },
  ],
  refreshMs: [
    { value: 1000, label: '1 second' },
    { value: 2000, label: '2 seconds' },
    { value: 5000, label: '5 seconds' },
    { value: 0, label: 'Paused' },
  ],
  historySeconds: [
    { value: 60, label: '1 minute' },
    { value: 300, label: '5 minutes' },
    { value: 600, label: '10 minutes' },
  ],
  closeBehaviour: [
    { value: 'hide', label: 'Hide to the notification area' },
    { value: 'quit', label: 'Quit osstat' },
  ],
  // The only setting whose choices carry a description of their own. The three
  // levels differ in what they *capture*, which a one-word label cannot say,
  // and the difference is the whole basis for picking one — so `Logging`
  // renders these itself rather than through `Choice`.
  logLevel: [
    {
      value: 'info',
      label: 'Info',
      description: 'Starting up, model sessions, downloads and their outcomes.',
    },
    {
      value: 'debug',
      label: 'Debug',
      description:
        'Adds each page you open, each setting you change, and every command that failed.',
    },
    {
      value: 'verbose',
      label: 'Verbose',
      description:
        'Adds a line for every measurement, one every couple of seconds. Turn this on to reproduce a problem rather than leaving it on.',
    },
  ],
} as const;

/** What a fresh install uses. */
export const DEFAULT_PREFERENCES: Preferences = {
  theme: DEFAULT_THEME,
  navigation: 'sidebar',
  pageLayout: 'onePage',
  refreshMs: 2000,
  historySeconds: 300,
  closeBehaviour: 'hide',
  logLevel: 'info',
  hasSeenTrayNotice: false,
  overviewPanels: [],
};

/**
 * Where preferences live.
 *
 * Exported because `public/theme-boot.js` has to read this same key before any
 * module exists to import it from, and a test compares the two spellings. A
 * theme stored under one key and read under another would leave the boot
 * script applying nothing, silently, forever.
 */
export const STORAGE_KEY = 'osstat.preferences.v1';

/** Whether `value` is one of the values `key` allows. */
function isAllowed<K extends keyof typeof CHOICES>(
  key: K,
  value: unknown
): value is Preferences[K] {
  return CHOICES[key].some((choice) => choice.value === value);
}

/**
 * Coerces arbitrary stored data into valid preferences.
 *
 * Every field falls back independently, so one unrecognised value — a setting
 * removed in a later version, or a hand-edited entry — costs only that setting
 * rather than resetting the rest. Reading preferences must never be able to
 * white-screen the app.
 *
 * @param raw Anything that came out of storage.
 * @returns Valid preferences, with defaults filling any gap.
 */
export function coercePreferences(raw: unknown): Preferences {
  if (typeof raw !== 'object' || raw === null) return { ...DEFAULT_PREFERENCES };

  const candidate = raw as Partial<Record<keyof Preferences, unknown>>;

  return {
    theme: isAllowed('theme', candidate.theme) ? candidate.theme : DEFAULT_PREFERENCES.theme,
    navigation: isAllowed('navigation', candidate.navigation)
      ? candidate.navigation
      : DEFAULT_PREFERENCES.navigation,
    pageLayout: isAllowed('pageLayout', candidate.pageLayout)
      ? candidate.pageLayout
      : DEFAULT_PREFERENCES.pageLayout,
    refreshMs: isAllowed('refreshMs', candidate.refreshMs)
      ? candidate.refreshMs
      : DEFAULT_PREFERENCES.refreshMs,
    historySeconds: isAllowed('historySeconds', candidate.historySeconds)
      ? candidate.historySeconds
      : DEFAULT_PREFERENCES.historySeconds,
    closeBehaviour: isAllowed('closeBehaviour', candidate.closeBehaviour)
      ? candidate.closeBehaviour
      : DEFAULT_PREFERENCES.closeBehaviour,
    // Falling back to Info rather than to whatever was stored matters more here
    // than for the other settings: an unrecognised value would otherwise be
    // pushed to the backend, and the quietest level is the safe direction to
    // fail in for a setting whose loudest one writes continuously.
    logLevel: isAllowed('logLevel', candidate.logLevel)
      ? candidate.logLevel
      : DEFAULT_PREFERENCES.logLevel,
    // A plain boolean rather than a CHOICES entry: it is a record of something
    // that happened, not a setting anyone picks.
    hasSeenTrayNotice: candidate.hasSeenTrayNotice === true,
    // Not an `isAllowed` check: this is a list, not one of a fixed set of
    // choices, so it has its own coercion that repairs entries individually.
    overviewPanels: coercePanelLayout(candidate.overviewPanels),
  };
}

/**
 * Reads stored preferences.
 *
 * @returns The stored preferences, or defaults if nothing valid is stored.
 */
export function loadPreferences(): Preferences {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    if (stored === null) return { ...DEFAULT_PREFERENCES };
    return coercePreferences(JSON.parse(stored));
  } catch {
    // Storage can be unavailable or the contents unparseable. Neither is worth
    // failing over for a layout choice.
    return { ...DEFAULT_PREFERENCES };
  }
}

/**
 * Writes preferences to storage, ignoring a storage failure.
 *
 * @param preferences The preferences to persist.
 */
export function savePreferences(preferences: Preferences): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(preferences));
  } catch {
    // A full or disabled storage means the choice does not survive a restart.
    // That is a far smaller problem than refusing to apply it now.
  }
}

/**
 * How many history samples a window covers at a given tick rate.
 *
 * @param historySeconds The configured window.
 * @param refreshMs The configured tick, where `0` means paused.
 * @returns A sample count of at least one.
 */
export function samplesInWindow(historySeconds: number, refreshMs: number): number {
  const tick = refreshMs > 0 ? refreshMs : DEFAULT_PREFERENCES.refreshMs;
  return Math.max(1, Math.ceil((historySeconds * 1000) / tick));
}

/**
 * Reads and updates the view preferences.
 *
 * @returns The current preferences and a function that merges a partial update.
 */
export function usePreferences(): [Preferences, (update: Partial<Preferences>) => void] {
  const [preferences, setPreferences] = useState<Preferences>(loadPreferences);

  useEffect(() => {
    savePreferences(preferences);
  }, [preferences]);

  const update = useCallback((change: Partial<Preferences>) => {
    setPreferences((current) => coercePreferences({ ...current, ...change }));
  }, []);

  return [preferences, update];
}
