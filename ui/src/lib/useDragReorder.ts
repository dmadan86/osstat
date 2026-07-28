/**
 * Reordering by dragging a grip.
 *
 * Written against pointer events rather than a drag library. The job is
 * reordering a handful of cards in a flow container, keyboard access is already
 * covered by the panel menu, and this follows `Chart.tsx`: hand-written because
 * a dependency would cost more than it saves.
 *
 * The gesture starts from a grip rather than anywhere on the panel because
 * panels hold charts with hover crosshairs — dragging to reorder and dragging
 * across a chart to read it would otherwise be the same gesture.
 */

import { useCallback, useEffect, useRef, useState } from 'react';

import { reorder } from './panelLayout';

/** What a consumer needs to wire up a draggable list. */
export interface DragReorder {
  /** The id currently being dragged, or `null`. */
  draggingId: string | null;
  /** Ref callback registering the element that represents `id`. */
  register: (id: string) => (element: HTMLElement | null) => void;
  /** Props for the grip that starts a drag of `id`. */
  gripProps: (id: string) => {
    onPointerDown: (event: React.PointerEvent) => void;
    role: 'button';
    tabIndex: -1;
    'aria-hidden': true;
  };
}

/**
 * Tracks a drag-to-reorder gesture.
 *
 * @param order The ids currently rendered, in render order.
 * @param onReorder Called with the new order when a drag ends somewhere new.
 * @returns The dragging state and the props that drive it.
 */
export function useDragReorder(
  order: readonly string[],
  onReorder: (next: string[]) => void
): DragReorder {
  const [draggingId, setDraggingId] = useState<string | null>(null);

  // Refs rather than state: these change during a gesture, and re-rendering on
  // every pointermove would be both wasteful and, for the element map, wrong.
  const elements = useRef(new Map<string, HTMLElement>());
  const latestOrder = useRef(order);
  const dropIndex = useRef<number | null>(null);

  useEffect(() => {
    latestOrder.current = order;
  }, [order]);

  const register = useCallback(
    (id: string) => (element: HTMLElement | null) => {
      if (element === null) elements.current.delete(id);
      else elements.current.set(id, element);
    },
    []
  );

  useEffect(() => {
    if (draggingId === null) return undefined;

    /** The index whose row currently contains `y`. */
    const indexAt = (y: number): number | null => {
      for (const [index, id] of latestOrder.current.entries()) {
        const rect = elements.current.get(id)?.getBoundingClientRect();
        if (rect === undefined) continue;
        if (y >= rect.top && y <= rect.bottom) return index;
      }
      return null;
    };

    const move = (event: PointerEvent): void => {
      dropIndex.current = indexAt(event.clientY);
    };

    const finish = (): void => {
      const from = latestOrder.current.indexOf(draggingId);
      const to = dropIndex.current;

      setDraggingId(null);
      dropIndex.current = null;

      if (to === null || from === -1 || from === to) return;
      onReorder(reorder(latestOrder.current, from, to));
    };

    const cancel = (event: KeyboardEvent): void => {
      if (event.key !== 'Escape') return;
      setDraggingId(null);
      dropIndex.current = null;
    };

    document.addEventListener('pointermove', move);
    document.addEventListener('pointerup', finish);
    document.addEventListener('keydown', cancel);

    return () => {
      document.removeEventListener('pointermove', move);
      document.removeEventListener('pointerup', finish);
      document.removeEventListener('keydown', cancel);
    };
  }, [draggingId, onReorder]);

  const gripProps = useCallback(
    (id: string) =>
      ({
        onPointerDown: (event: React.PointerEvent) => {
          event.preventDefault();
          dropIndex.current = latestOrder.current.indexOf(id);
          setDraggingId(id);
        },
        role: 'button',
        tabIndex: -1,
        'aria-hidden': true,
      }) as const,
    []
  );

  return { draggingId, register, gripProps };
}
