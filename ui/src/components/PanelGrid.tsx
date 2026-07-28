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
  moveById,
  NARROW_WIDTH,
  PANEL_HEIGHTS,
  updatePanel,
  type PanelLayout,
} from '../lib/panelLayout';

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
            onLayoutChange(moveById(layout, panel.id, delta));
          },
          onHide: () => {
            onLayoutChange(updatePanel(layout, panel.id, { hidden: true }));
          },
        };

        return (
          <div
            key={panel.id}
            data-testid={`panel-${panel.id}`}
            style={{ gridColumn: `span ${narrow ? 12 : panel.span}` }}
          >
            <Collapsible section={section} defaultOpen controls={controls} />
          </div>
        );
      })}
    </div>
  );
}

/** The chart height a named panel height is worth. */
export function chartHeightFor(layout: readonly PanelLayout[], id: string): number {
  const panel = layout.find((candidate) => candidate.id === id);
  return PANEL_HEIGHTS[panel?.height ?? 'normal'];
}
