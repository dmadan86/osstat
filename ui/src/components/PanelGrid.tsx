/**
 * The Overview's panels, arranged.
 *
 * A twelve-column flow grid: panels wrap rather than holding absolute
 * positions, so an arrangement can never contain a hole the user has to tidy
 * up, and a window too narrow for columns reflows instead of clipping.
 */

import { useEffect, useRef, useState } from 'react';

import { Collapsible, type SectionSpec } from './Panel';
import type { PanelControls } from './PanelMenu';
import {
  applyVisibleOrder,
  moveVisibleById,
  NARROW_WIDTH,
  updatePanel,
  type PanelLayout,
} from '../lib/panelLayout';
import { useDragReorder } from '../lib/useDragReorder';

/** What the grid needs. */
export interface PanelGridProps {
  /** The sections available to render. */
  sections: SectionSpec[];
  /** The reconciled layout, in render order, including hidden panels. */
  layout: PanelLayout[];
  /** Applies an arrangement change. */
  onLayoutChange: (next: PanelLayout[]) => void;
}

/**
 * Reports an element's content width, remeasured as it changes.
 *
 * Measured from the container rather than the viewport because the sidebar's
 * width counts against the panels, and a media query cannot see that.
 *
 * @param target The element to measure.
 * @returns The last measured width, or `null` before the first measurement.
 */
function useContentWidth(target: React.RefObject<HTMLElement | null>): number | null {
  const [width, setWidth] = useState<number | null>(null);

  useEffect(() => {
    const element = target.current;
    if (element === null) return undefined;

    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (entry !== undefined) setWidth(entry.contentRect.width);
    });

    observer.observe(element);
    return () => {
      observer.disconnect();
    };
  }, [target]);

  return width;
}

/**
 * Renders the arranged panels.
 *
 * @param props The sections, the layout and the change handler.
 */
export function PanelGrid({ sections, layout, onLayoutChange }: PanelGridProps): React.JSX.Element {
  const container = useRef<HTMLDivElement | null>(null);
  const width = useContentWidth(container);

  // Before the first measurement, assume there is room. Assuming the opposite
  // would make every panel flash full-width on mount.
  const narrow = width !== null && width < NARROW_WIDTH;

  const visible = layout.filter((panel) => !panel.hidden);
  const visibleIds = visible.map((panel) => panel.id);

  const drag = useDragReorder(visibleIds, (next) => {
    // Reordering the visible panels must not disturb the hidden ones. Task 5
    // added `applyVisibleOrder` to `panelLayout.ts` for exactly this, after the
    // menu's Move up/Move down hit the same problem: a hidden panel between two
    // visible ones absorbed the move and the click looked like a no-op. Reuse it
    // rather than writing a second translation that can drift from the first.
    onLayoutChange(applyVisibleOrder(layout, next));
  });

  return (
    <div ref={container} className="grid grid-cols-12 items-start gap-2">
      {visible.length === 0 && (
        <p role="status" className="col-span-12 py-8 text-center text-sm text-neutral-500">
          Every panel is hidden. Bring them back from Settings › Panels.
        </p>
      )}

      {visible.map((panel, index) => {
        const section = sections.find((candidate) => candidate.id === panel.id);
        if (section === undefined) return null;

        const controls: PanelControls = {
          span: panel.span,
          height: panel.height,
          canMoveUp: index > 0,
          canMoveDown: index < visible.length - 1,
          showSize: true,
          onSpan: (span) => {
            onLayoutChange(updatePanel(layout, panel.id, { span }));
          },
          onHeight: (height) => {
            onLayoutChange(updatePanel(layout, panel.id, { height }));
          },
          onMove: (delta) => {
            onLayoutChange(moveVisibleById(layout, panel.id, delta));
          },
          onHide: () => {
            onLayoutChange(updatePanel(layout, panel.id, { hidden: true }));
          },
        };

        return (
          <div
            key={panel.id}
            ref={drag.register(panel.id)}
            data-testid={`panel-${panel.id}`}
            style={{ gridColumn: `span ${narrow ? 12 : panel.span}` }}
            className={drag.draggingId === panel.id ? 'opacity-50' : undefined}
          >
            <div
              // Charts capture the pointer for their crosshair. During a drag
              // that would swallow the gesture the moment the pointer crossed
              // one, so bodies stop listening until the drag ends.
              className={drag.draggingId === null ? undefined : '[&_[id]]:pointer-events-none'}
            >
              <Collapsible
                section={section}
                defaultOpen
                controls={controls}
                grip={
                  <span
                    {...drag.gripProps(panel.id)}
                    title={`Drag to reorder ${section.title}`}
                    className="cursor-grab select-none px-1.5 py-2 text-neutral-600 hover:text-neutral-300"
                  >
                    ⠿
                  </span>
                }
              />
            </div>
          </div>
        );
      })}
    </div>
  );
}
