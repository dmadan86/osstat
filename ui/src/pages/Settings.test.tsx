import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { Settings } from './Settings';
import { DEFAULT_PREFERENCES, type Preferences } from '../lib/preferences';

function prefs(overrides: Partial<Preferences> = {}): Preferences {
  return { ...DEFAULT_PREFERENCES, ...overrides };
}

/**
 * Renders the page and opens the group holding the panel list.
 *
 * The tab is opened by clicking it rather than by a prop, so these tests still
 * fail if the Panels row is moved to a group that never renders it.
 */
function showMonitoring(preferences: Preferences, onChange: () => void): void {
  render(<Settings preferences={preferences} onChange={onChange} />);
  fireEvent.click(screen.getByRole('tab', { name: 'Monitoring' }));
}

describe('Settings › Panels', () => {
  it('lists every Overview panel', () => {
    showMonitoring(prefs(), vi.fn());
    for (const title of ['CPU', 'Memory', 'Disks', 'Network', 'GPU']) {
      expect(screen.getByRole('checkbox', { name: title })).toBeInTheDocument();
    }
  });

  it('shows a panel as ticked when it is visible', () => {
    showMonitoring(prefs(), vi.fn());
    expect(screen.getByRole('checkbox', { name: 'GPU' })).toBeChecked();
  });

  it('shows a hidden panel as unticked', () => {
    const preferences = prefs({
      overviewPanels: [{ id: 'gpu', span: 12, height: 'normal', hidden: true }],
    });
    showMonitoring(preferences, vi.fn());
    expect(screen.getByRole('checkbox', { name: 'GPU' })).not.toBeChecked();
  });

  it('brings a hidden panel back', () => {
    const onChange = vi.fn();
    const preferences = prefs({
      overviewPanels: [{ id: 'gpu', span: 12, height: 'normal', hidden: true }],
    });
    showMonitoring(preferences, onChange);

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
    showMonitoring(preferences, onChange);

    fireEvent.click(screen.getByRole('button', { name: /reset overview layout/i }));

    expect(onChange).toHaveBeenCalledWith({ overviewPanels: [] });
  });
});

describe('Settings › groups', () => {
  it('opens on Appearance, so the page is never a blank card', () => {
    render(<Settings preferences={prefs()} onChange={vi.fn()} />);

    expect(screen.getByRole('tab', { name: 'Appearance' })).toHaveAttribute(
      'aria-selected',
      'true'
    );
    expect(screen.getByRole('radiogroup', { name: 'Theme' })).toBeInTheDocument();
  });

  it('puts every setting in exactly one group', () => {
    // The grouping is the whole point of the tabs, so the thing worth asserting
    // is that nothing fell between two of them. Each row is looked up by the
    // accessible name of its own control.
    const found = new Map<string, string[]>();

    for (const group of ['Appearance', 'Monitoring', 'Models', 'System']) {
      render(<Settings preferences={prefs()} onChange={vi.fn()} />);
      fireEvent.click(screen.getByRole('tab', { name: group }));

      for (const setting of [
        'Theme',
        'Navigation',
        'Page layout',
        'Refresh interval',
        'History window',
        'When I close the window',
      ]) {
        if (screen.queryByRole('radiogroup', { name: setting }) !== null) {
          found.set(setting, [...(found.get(setting) ?? []), group]);
        }
      }

      cleanup();
    }

    expect(Object.fromEntries(found)).toEqual({
      Theme: ['Appearance'],
      Navigation: ['Appearance'],
      'Page layout': ['Appearance'],
      'Refresh interval': ['Monitoring'],
      'History window': ['Monitoring'],
      'When I close the window': ['System'],
    });
  });

  it('moves between groups with the arrow keys, as a tablist promises', () => {
    render(<Settings preferences={prefs()} onChange={vi.fn()} />);

    const appearance = screen.getByRole('tab', { name: 'Appearance' });
    appearance.focus();
    fireEvent.keyDown(appearance, { key: 'ArrowRight' });

    const monitoring = screen.getByRole('tab', { name: 'Monitoring' });
    expect(monitoring).toHaveAttribute('aria-selected', 'true');
    // The selection and the focus move together, or the next arrow press starts
    // from somewhere the user cannot see.
    expect(monitoring).toHaveFocus();
  });

  it('wraps from the last group back to the first', () => {
    render(<Settings preferences={prefs()} onChange={vi.fn()} />);

    const appearance = screen.getByRole('tab', { name: 'Appearance' });
    appearance.focus();
    fireEvent.keyDown(appearance, { key: 'ArrowLeft' });

    expect(screen.getByRole('tab', { name: 'System' })).toHaveAttribute('aria-selected', 'true');
  });

  it('keeps one tab stop for the whole strip', () => {
    // A roving tab stop: Tab reaches the group bar once and lands on the open
    // group, rather than walking through four buttons to get past it.
    render(<Settings preferences={prefs()} onChange={vi.fn()} />);

    const reachable = screen
      .getAllByRole('tab')
      .filter((tab) => tab.getAttribute('tabindex') !== '-1');

    expect(reachable).toHaveLength(1);
    expect(reachable[0]).toHaveAccessibleName('Appearance');
  });
});
