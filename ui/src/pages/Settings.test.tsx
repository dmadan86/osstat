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
