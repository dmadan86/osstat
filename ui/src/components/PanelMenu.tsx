/**
 * The per-panel arrangement menu.
 *
 * Every way of arranging a panel lives here, including the two — Move up and
 * Move down — that dragging also provides. That redundancy is the point:
 * because the menu is complete, dragging can stay a pointer-only enhancement
 * and no keyboard user is ever asked to perform one.
 */

import { useEffect, useId, useRef, useState } from 'react';

import {
  HEIGHT_LABELS,
  PANEL_HEIGHT_NAMES,
  PANEL_SPANS,
  SPAN_LABELS,
  type PanelHeight,
  type PanelSpan,
} from '../lib/panelLayout';

/** What a panel can be told to do about its own placement. */
export interface PanelControls {
  /** Current width. */
  span: PanelSpan;
  /** Current height. */
  height: PanelHeight;
  /** Whether there is anywhere above to move to. */
  canMoveUp: boolean;
  /** Whether there is anywhere below to move to. */
  canMoveDown: boolean;
  /** Whether width and height apply at all. False under the sub-tabs layout. */
  showSize: boolean;
  /** Sets the width. */
  onSpan: (span: PanelSpan) => void;
  /** Sets the height. */
  onHeight: (height: PanelHeight) => void;
  /** Moves the panel one place; `-1` is up. */
  onMove: (delta: -1 | 1) => void;
  /** Hides the panel. */
  onHide: () => void;
}

/** A group heading inside the menu. */
function Group({ children }: { children: string }): React.JSX.Element {
  return (
    <p className="px-3 pb-1 pt-2 text-[10px] uppercase tracking-wider text-text-faint">
      {children}
    </p>
  );
}

/** One row of the menu. */
const ROW =
  'block w-full px-3 py-1.5 text-left text-xs hover:bg-white/[0.06] disabled:opacity-40 disabled:hover:bg-transparent';

/**
 * Renders the arrangement menu for one panel.
 *
 * @param props The panel's title, for labelling, and its controls.
 */
export function PanelMenu({
  title,
  span,
  height,
  canMoveUp,
  canMoveDown,
  showSize,
  onSpan,
  onHeight,
  onMove,
  onHide,
}: { title: string } & PanelControls): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const container = useRef<HTMLDivElement | null>(null);
  const menuId = useId();

  useEffect(() => {
    if (!open) return undefined;

    const dismiss = (event: Event): void => {
      if (event instanceof KeyboardEvent && event.key !== 'Escape') return;
      if (event.type === 'pointerdown' && event.target instanceof Node) {
        if (container.current?.contains(event.target) === true) return;
      }
      setOpen(false);
    };

    document.addEventListener('keydown', dismiss);
    document.addEventListener('pointerdown', dismiss);

    return () => {
      document.removeEventListener('keydown', dismiss);
      document.removeEventListener('pointerdown', dismiss);
    };
  }, [open]);

  return (
    <div ref={container} className="relative">
      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        aria-label={`Arrange ${title}`}
        onClick={() => {
          setOpen((current) => !current);
        }}
        className="rounded px-1.5 py-1 text-text-muted hover:bg-white/[0.06] hover:text-text"
      >
        <span aria-hidden="true">⋮</span>
      </button>

      {open && (
        <div
          id={menuId}
          role="menu"
          aria-label={`Arrange ${title}`}
          className="absolute right-0 z-20 mt-1 w-44 overflow-hidden rounded-lg border border-edge bg-surface-raised py-1 shadow-lg"
        >
          {showSize && (
            <>
              <Group>Width</Group>
              {PANEL_SPANS.map((candidate) => (
                <button
                  key={candidate}
                  type="button"
                  role="menuitemradio"
                  aria-checked={candidate === span}
                  className={`${ROW} ${candidate === span ? 'text-accent' : 'text-text'}`}
                  onClick={() => {
                    onSpan(candidate);
                    setOpen(false);
                  }}
                >
                  {SPAN_LABELS[candidate]}
                </button>
              ))}

              <Group>Height</Group>
              {PANEL_HEIGHT_NAMES.map((candidate) => (
                <button
                  key={candidate}
                  type="button"
                  role="menuitemradio"
                  aria-checked={candidate === height}
                  className={`${ROW} ${candidate === height ? 'text-accent' : 'text-text'}`}
                  onClick={() => {
                    onHeight(candidate);
                    setOpen(false);
                  }}
                >
                  {HEIGHT_LABELS[candidate]}
                </button>
              ))}
            </>
          )}

          <Group>Order</Group>
          <button
            type="button"
            role="menuitem"
            disabled={!canMoveUp}
            className={`${ROW} text-text`}
            onClick={() => {
              onMove(-1);
              setOpen(false);
            }}
          >
            Move up
          </button>
          <button
            type="button"
            role="menuitem"
            disabled={!canMoveDown}
            className={`${ROW} text-text`}
            onClick={() => {
              onMove(1);
              setOpen(false);
            }}
          >
            Move down
          </button>

          <div className="my-1 border-t border-edge" />
          <button
            type="button"
            role="menuitem"
            className={`${ROW} text-text`}
            onClick={() => {
              onHide();
              setOpen(false);
            }}
          >
            Hide this panel
          </button>
        </div>
      )}
    </div>
  );
}
