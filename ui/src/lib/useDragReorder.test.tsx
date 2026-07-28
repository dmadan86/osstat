import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { useDragReorder } from './useDragReorder';

/** A minimal consumer, so the hook is tested through its real interface. */
function Harness({ onReorder }: { onReorder: (next: string[]) => void }): React.JSX.Element {
  const drag = useDragReorder(['a', 'b', 'c'], onReorder);

  return (
    <div>
      {['a', 'b', 'c'].map((id, index) => (
        <div key={id} ref={drag.register(id)} data-testid={`row-${id}`}>
          <span data-testid={`grip-${id}`} {...drag.gripProps(id)} />
          <span>{drag.draggingId === id ? `${id} dragging` : id}</span>
          <span data-testid={`index-${id}`}>{index}</span>
        </div>
      ))}
    </div>
  );
}

/** jsdom lays nothing out, so rectangles are supplied. */
function stackRows(): void {
  for (const [id, top] of [
    ['a', 0],
    ['b', 100],
    ['c', 200],
  ] as const) {
    screen.getByTestId(`row-${id}`).getBoundingClientRect = () =>
      ({
        top,
        bottom: top + 100,
        left: 0,
        right: 400,
        height: 100,
        width: 400,
        x: 0,
        y: top,
      }) as DOMRect;
  }
}

describe('useDragReorder', () => {
  it('does nothing until a drag starts', () => {
    render(<Harness onReorder={vi.fn()} />);
    expect(screen.queryByText(/dragging/)).not.toBeInTheDocument();
  });

  it('marks the dragged panel while the pointer is down', () => {
    render(<Harness onReorder={vi.fn()} />);
    stackRows();
    fireEvent.pointerDown(screen.getByTestId('grip-a'), { clientY: 10, pointerId: 1 });
    expect(screen.getByText('a dragging')).toBeInTheDocument();
  });

  it('reorders on drop, by the row the pointer ended over', () => {
    const onReorder = vi.fn();
    render(<Harness onReorder={onReorder} />);
    stackRows();

    fireEvent.pointerDown(screen.getByTestId('grip-a'), { clientY: 10, pointerId: 1 });
    fireEvent.pointerMove(document, { clientY: 250, pointerId: 1 });
    fireEvent.pointerUp(document, { clientY: 250, pointerId: 1 });

    expect(onReorder).toHaveBeenCalledWith(['b', 'c', 'a']);
  });

  it('abandons the drag on Escape without reordering', () => {
    const onReorder = vi.fn();
    render(<Harness onReorder={onReorder} />);
    stackRows();

    fireEvent.pointerDown(screen.getByTestId('grip-a'), { clientY: 10, pointerId: 1 });
    fireEvent.pointerMove(document, { clientY: 250, pointerId: 1 });
    fireEvent.keyDown(document, { key: 'Escape' });

    expect(onReorder).not.toHaveBeenCalled();
    expect(screen.queryByText(/dragging/)).not.toBeInTheDocument();
  });

  it('does not report a reorder when the panel is dropped where it started', () => {
    const onReorder = vi.fn();
    render(<Harness onReorder={onReorder} />);
    stackRows();

    fireEvent.pointerDown(screen.getByTestId('grip-b'), { clientY: 110, pointerId: 1 });
    fireEvent.pointerUp(document, { clientY: 115, pointerId: 1 });

    expect(onReorder).not.toHaveBeenCalled();
  });
});
