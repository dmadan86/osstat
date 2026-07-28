import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { Collapsible, type SectionSpec } from './Panel';
import type { PanelControls } from './PanelMenu';

function section(): SectionSpec {
  return { id: 'cpu', title: 'CPU', content: <p>Body</p> };
}

function controls(): PanelControls {
  return {
    span: 12,
    height: 'normal',
    canMoveUp: true,
    canMoveDown: true,
    showSize: true,
    onSpan: vi.fn(),
    onHeight: vi.fn(),
    onMove: vi.fn(),
    onHide: vi.fn(),
  };
}

describe('Collapsible', () => {
  it('opens the arrangement menu with nothing clipping it between the menu and the card', () => {
    // Regression guard for the dropdown-clipping bug: `overflow-hidden` on the
    // <section> (or any other ancestor between the menu and the card) clips
    // an absolutely-positioned descendant regardless of its own positioning.
    // jsdom does not lay out overflow, so this asserts the structure instead:
    // no ancestor between the open menu and the outer <section> may carry
    // `overflow-hidden`. If this fails, someone reintroduced it without
    // seeing why it had to move.
    render(<Collapsible section={section()} defaultOpen controls={controls()} />);
    fireEvent.click(screen.getByRole('button', { name: /arrange cpu/i }));

    const menu = screen.getByRole('menu');
    expect(menu).toBeInTheDocument();

    for (let ancestor = menu.parentElement; ancestor !== null; ancestor = ancestor.parentElement) {
      expect(ancestor.className).not.toMatch(/\boverflow-hidden\b/);
      if (ancestor.tagName === 'SECTION') break;
    }
  });
});
