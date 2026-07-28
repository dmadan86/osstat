import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { PanelGrid } from './PanelGrid';
import { defaultPanelLayout, type PanelLayout } from '../lib/panelLayout';
import type { SectionSpec } from './Section';

const SECTIONS: SectionSpec[] = [
  { id: 'cpu', title: 'CPU', content: <p>cpu body</p> },
  { id: 'memory', title: 'Memory', content: <p>memory body</p> },
  { id: 'disks', title: 'Disks', content: <p>disks body</p> },
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

  it('moving a panel steps over a hidden neighbour in one click', async () => {
    const user = userEvent.setup();
    const onLayoutChange = vi.fn();

    // disks sits hidden between the two visible panels: cpu, disks(hidden),
    // memory. A move that only counted array positions (moveById) would swap
    // memory with the hidden disks and leave the visible order -- cpu, memory
    // -- unchanged, so the click would look like it did nothing.
    render(
      <PanelGrid
        sections={SECTIONS}
        layout={layout({ id: 'cpu' }, { id: 'disks', hidden: true }, { id: 'memory' })}
        onLayoutChange={onLayoutChange}
      />
    );

    await user.click(screen.getByRole('button', { name: 'Arrange Memory' }));
    await user.click(screen.getByRole('menuitem', { name: 'Move up' }));

    expect(onLayoutChange).toHaveBeenCalledTimes(1);
    const next = onLayoutChange.mock.calls[0]?.[0] as PanelLayout[];

    expect(next.map((panel) => panel.id)).toEqual(['memory', 'disks', 'cpu']);
    expect(next.filter((panel) => !panel.hidden).map((panel) => panel.id)).toEqual([
      'memory',
      'cpu',
    ]);
    // The hidden panel kept its own slot rather than absorbing the move.
    expect(next[1]).toMatchObject({ id: 'disks', hidden: true });
  });

  it('reorders by dragging a grip, without disturbing a hidden neighbour', () => {
    const onLayoutChange = vi.fn();

    // Same arrangement as the menu case above: cpu, disks(hidden), memory.
    // Dragging memory to the top must produce memory, disks, cpu -- not swap
    // memory with the hidden disks and leave the visible order unchanged.
    render(
      <PanelGrid
        sections={SECTIONS}
        layout={layout({ id: 'cpu' }, { id: 'disks', hidden: true }, { id: 'memory' })}
        onLayoutChange={onLayoutChange}
      />
    );

    const cpuRow = screen.getByTestId('panel-cpu');
    const memoryRow = screen.getByTestId('panel-memory');

    cpuRow.getBoundingClientRect = () =>
      ({
        top: 0,
        bottom: 100,
        left: 0,
        right: 400,
        height: 100,
        width: 400,
        x: 0,
        y: 0,
      }) as DOMRect;
    memoryRow.getBoundingClientRect = () =>
      ({
        top: 100,
        bottom: 200,
        left: 0,
        right: 400,
        height: 100,
        width: 400,
        x: 0,
        y: 100,
      }) as DOMRect;

    fireEvent.pointerDown(screen.getByTitle('Drag to reorder Memory'), {
      clientY: 150,
      pointerId: 1,
    });
    fireEvent.pointerMove(document, { clientY: 10, pointerId: 1 });
    fireEvent.pointerUp(document, { clientY: 10, pointerId: 1 });

    expect(onLayoutChange).toHaveBeenCalledTimes(1);
    const next = onLayoutChange.mock.calls[0]?.[0] as PanelLayout[];

    expect(next.map((panel) => panel.id)).toEqual(['memory', 'disks', 'cpu']);
    expect(next.filter((panel) => !panel.hidden).map((panel) => panel.id)).toEqual([
      'memory',
      'cpu',
    ]);
    expect(next[1]).toMatchObject({ id: 'disks', hidden: true });
  });
});
