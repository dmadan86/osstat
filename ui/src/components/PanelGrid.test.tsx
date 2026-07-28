import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { PanelGrid } from './PanelGrid';
import { defaultPanelLayout, type PanelLayout } from '../lib/panelLayout';
import type { SectionSpec } from './Section';

const SECTIONS: SectionSpec[] = [
  { id: 'cpu', title: 'CPU', content: <p>cpu body</p> },
  { id: 'memory', title: 'Memory', content: <p>memory body</p> },
];

function layout(...entries: Array<Partial<PanelLayout> & { id: string }>): PanelLayout[] {
  return entries.map((entry) => ({ ...defaultPanelLayout(entry.id), ...entry }));
}

/** Reports a fixed content width, since jsdom lays nothing out. */
function widthIs(pixels: number): void {
  vi.stubGlobal(
    'ResizeObserver',
    class {
      constructor(private readonly callback: ResizeObserverCallback) {}
      observe(target: Element): void {
        this.callback(
          [{ target, contentRect: { width: pixels } } as unknown as ResizeObserverEntry],
          this as unknown as ResizeObserver
        );
      }
      unobserve(): void {}
      disconnect(): void {}
    }
  );
}

describe('PanelGrid', () => {
  beforeEach(() => {
    widthIs(1200);
  });

  it('renders panels in the layout order, not the section order', () => {
    render(
      <PanelGrid
        sections={SECTIONS}
        layout={layout({ id: 'memory' }, { id: 'cpu' })}
        onLayoutChange={vi.fn()}
      />
    );

    const headings = screen.getAllByRole('heading', { level: 3 });
    expect(headings.map((heading) => heading.textContent)).toEqual([
      expect.stringContaining('Memory'),
      expect.stringContaining('CPU'),
    ]);
  });

  it('gives each panel its stored width', () => {
    render(
      <PanelGrid
        sections={SECTIONS}
        layout={layout({ id: 'cpu', span: 4 }, { id: 'memory', span: 8 })}
        onLayoutChange={vi.fn()}
      />
    );

    expect(screen.getByTestId('panel-cpu')).toHaveStyle({ gridColumn: 'span 4' });
    expect(screen.getByTestId('panel-memory')).toHaveStyle({ gridColumn: 'span 8' });
  });

  it('does not render a hidden panel', () => {
    render(
      <PanelGrid
        sections={SECTIONS}
        layout={layout({ id: 'cpu', hidden: true }, { id: 'memory' })}
        onLayoutChange={vi.fn()}
      />
    );

    expect(screen.queryByText('cpu body')).not.toBeInTheDocument();
    expect(screen.getByText('memory body')).toBeInTheDocument();
  });

  it('says how to get panels back when every one is hidden', () => {
    render(
      <PanelGrid
        sections={SECTIONS}
        layout={layout({ id: 'cpu', hidden: true }, { id: 'memory', hidden: true })}
        onLayoutChange={vi.fn()}
      />
    );

    expect(screen.getByRole('status')).toHaveTextContent(/settings/i);
  });

  it('gives every panel the full row when there is not enough width for columns', () => {
    // The stored span is not modified -- it applies again when there is room.
    widthIs(700);
    render(
      <PanelGrid
        sections={SECTIONS}
        layout={layout({ id: 'cpu', span: 4 }, { id: 'memory', span: 4 })}
        onLayoutChange={vi.fn()}
      />
    );

    expect(screen.getByTestId('panel-cpu')).toHaveStyle({ gridColumn: 'span 12' });
  });
});
