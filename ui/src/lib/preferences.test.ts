import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  coercePreferences,
  DEFAULT_PREFERENCES,
  loadPreferences,
  samplesInWindow,
  savePreferences,
  type Preferences,
} from './preferences';

describe('coercePreferences', () => {
  it('accepts a fully valid object unchanged', () => {
    const valid = {
      navigation: 'tabs',
      pageLayout: 'subTabs',
      refreshMs: 5000,
      historySeconds: 60,
      overviewPanels: [],
    };

    expect(coercePreferences(valid)).toEqual(valid);
  });

  it('falls back per field, so one bad value does not reset the rest', () => {
    const partly = {
      navigation: 'rail',
      pageLayout: 'nonsense',
      refreshMs: 1000,
      historySeconds: 'later',
    };

    const result = coercePreferences(partly);

    expect(result.navigation).toBe('rail');
    expect(result.refreshMs).toBe(1000);
    expect(result.pageLayout).toBe(DEFAULT_PREFERENCES.pageLayout);
    expect(result.historySeconds).toBe(DEFAULT_PREFERENCES.historySeconds);
  });

  it('rejects a refresh interval that is not one the UI offers', () => {
    // A hand-edited or stale value must not become the sampler's tick rate.
    expect(coercePreferences({ refreshMs: 7 }).refreshMs).toBe(DEFAULT_PREFERENCES.refreshMs);
  });

  it('accepts the paused interval, which is a legitimate choice', () => {
    expect(coercePreferences({ refreshMs: 0 }).refreshMs).toBe(0);
  });

  it.each([null, undefined, 42, 'a string', []])('falls back entirely for %p', (raw) => {
    expect(coercePreferences(raw)).toEqual(DEFAULT_PREFERENCES);
  });
});

describe('loadPreferences', () => {
  beforeEach(() => {
    window.localStorage.clear();
    vi.restoreAllMocks();
  });

  it('returns defaults when nothing is stored', () => {
    expect(loadPreferences()).toEqual(DEFAULT_PREFERENCES);
  });

  it('round-trips through storage', () => {
    const chosen: Preferences = {
      navigation: 'rail',
      pageLayout: 'subTabs',
      refreshMs: 1000,
      historySeconds: 600,
      overviewPanels: [],
    };

    savePreferences(chosen);

    expect(loadPreferences()).toEqual(chosen);
  });

  it('returns defaults rather than throwing when storage holds garbage', () => {
    window.localStorage.setItem('osstat.preferences.v1', '{not json');

    expect(loadPreferences()).toEqual(DEFAULT_PREFERENCES);
  });

  it('returns defaults rather than throwing when storage is unavailable', () => {
    // A preference read must never be able to white-screen the app.
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('storage disabled');
    });

    expect(loadPreferences()).toEqual(DEFAULT_PREFERENCES);
  });

  it('does not throw when storage rejects a write', () => {
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('quota exceeded');
    });

    expect(() => {
      savePreferences(DEFAULT_PREFERENCES);
    }).not.toThrow();
  });
});

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

describe('samplesInWindow', () => {
  it('covers the window at the chosen tick rate', () => {
    expect(samplesInWindow(300, 2000)).toBe(150);
    expect(samplesInWindow(60, 1000)).toBe(60);
    expect(samplesInWindow(600, 5000)).toBe(120);
  });

  it('falls back to the default tick when sampling is paused', () => {
    // Dividing by a zero interval would ask for an infinite window.
    expect(Number.isFinite(samplesInWindow(300, 0))).toBe(true);
    expect(samplesInWindow(300, 0)).toBe(150);
  });

  it('never asks for fewer than one sample', () => {
    expect(samplesInWindow(0, 2000)).toBe(1);
  });
});
