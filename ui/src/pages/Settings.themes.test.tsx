import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { Settings } from './Settings';
import { DEFAULT_PREFERENCES, type Preferences } from '../lib/preferences';
import { THEMES } from '../lib/theme';

function prefs(overrides: Partial<Preferences> = {}): Preferences {
  return { ...DEFAULT_PREFERENCES, ...overrides };
}

/** The swatch inside one theme's row, which is the row's own preview. */
function swatchIn(row: HTMLElement): Element | null {
  return row.querySelector('[data-theme]');
}

describe('Settings › Theme', () => {
  it('offers every theme, by name and by what it is for', () => {
    render(<Settings preferences={prefs()} onChange={vi.fn()} />);

    for (const theme of THEMES) {
      const row = screen.getByRole('radio', { name: new RegExp(theme.label, 'i') });
      expect(row).toHaveTextContent(theme.description);
    }
  });

  it('marks the stored theme as the chosen one, and only that one', () => {
    render(<Settings preferences={prefs({ theme: 'terminal' })} onChange={vi.fn()} />);

    // Scoped to this radiogroup: the page carries five others, and every one
    // of them has a chosen option too.
    const chosen = within(screen.getByRole('radiogroup', { name: 'Theme' }))
      .getAllByRole('radio')
      .filter((radio) => radio.getAttribute('aria-checked') === 'true');

    expect(chosen).toHaveLength(1);
    expect(chosen[0]).toHaveTextContent('Terminal');
  });

  it('asks for the theme that was clicked', () => {
    const onChange = vi.fn();
    render(<Settings preferences={prefs()} onChange={onChange} />);

    fireEvent.click(screen.getByRole('radio', { name: /carbon/i }));

    expect(onChange).toHaveBeenCalledWith({ theme: 'carbon' });
  });

  it('previews each theme with a swatch wearing that theme, not a copied colour', () => {
    // The swatch draws `bg-surface-raised` and `bg-accent` through its own
    // `data-theme`, so it cannot show a colour the theme does not have. This
    // asserts the wiring; `tokens.test.ts` asserts the colours behind it.
    render(<Settings preferences={prefs()} onChange={vi.fn()} />);

    for (const theme of THEMES) {
      const row = screen.getByRole('radio', { name: new RegExp(theme.label, 'i') });
      expect(swatchIn(row)).toHaveAttribute('data-theme', theme.value);
    }
  });

  it('says outright that none of the four is a light theme', () => {
    // The row offers four dark themes and no light one. Leaving that unsaid
    // makes it look like an omission rather than the decision it is.
    render(<Settings preferences={prefs()} onChange={vi.fn()} />);

    expect(screen.getByRole('radiogroup', { name: 'Theme' }).parentElement).toHaveTextContent(
      /all four are dark/i
    );
  });
});
