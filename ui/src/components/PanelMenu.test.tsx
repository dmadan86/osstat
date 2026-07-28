import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { PanelMenu, type PanelControls } from './PanelMenu';

function controls(overrides: Partial<PanelControls> = {}): PanelControls {
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
    ...overrides,
  };
}

function open(props: PanelControls): void {
  render(<PanelMenu title="CPU" {...props} />);
  fireEvent.click(screen.getByRole('button', { name: /arrange cpu/i }));
}

describe('PanelMenu', () => {
  it('stays closed until asked', () => {
    render(<PanelMenu title="CPU" {...controls()} />);
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });

  it('reports the current width so the choice is visible, not just remembered', () => {
    open(controls({ span: 6 }));
    expect(screen.getByRole('menuitemradio', { name: 'Half' })).toHaveAttribute(
      'aria-checked',
      'true'
    );
  });

  it('changes the width', () => {
    const props = controls();
    open(props);
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Third' }));
    expect(props.onSpan).toHaveBeenCalledWith(4);
  });

  it('changes the height', () => {
    const props = controls();
    open(props);
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Tall' }));
    expect(props.onHeight).toHaveBeenCalledWith('tall');
  });

  it('reorders without a mouse drag', () => {
    const props = controls();
    open(props);
    fireEvent.click(screen.getByRole('menuitem', { name: /move up/i }));
    expect(props.onMove).toHaveBeenCalledWith(-1);
  });

  it('disables moving past either end rather than silently doing nothing', () => {
    open(controls({ canMoveUp: false }));
    expect(screen.getByRole('menuitem', { name: /move up/i })).toBeDisabled();
  });

  it('hides the panel', () => {
    const props = controls();
    open(props);
    fireEvent.click(screen.getByRole('menuitem', { name: /hide/i }));
    expect(props.onHide).toHaveBeenCalled();
  });

  it('omits size when one section fills the pane', () => {
    // In sub-tabs layout width and height mean nothing, and offering a control
    // that does nothing is worse than omitting it.
    open(controls({ showSize: false }));
    expect(screen.queryByRole('menuitemradio', { name: 'Half' })).not.toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: /move up/i })).toBeInTheDocument();
  });

  it('closes on Escape', () => {
    open(controls());
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });
});
