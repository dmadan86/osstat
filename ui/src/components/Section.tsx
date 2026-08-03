/**
 * A page section, and the container that decides how sections are presented.
 *
 * The two view settings are independent, and this is half of why that is cheap:
 * a section knows nothing about whether it is stacked or tabbed. The container
 * renders the same components either way.
 */

import { useState } from 'react';

import { Collapsible, type SectionSpec } from './Panel';
import { PanelGrid } from './PanelGrid';
import { PanelMenu } from './PanelMenu';
import { moveVisibleById, updatePanel, type PanelLayout } from '../lib/panelLayout';

export type { SectionSpec } from './Panel';

/** How a page presents its sections. */
export interface SectionContainerProps {
  /** The sections, in canonical order. */
  sections: SectionSpec[];
  /** `onePage` arranges them in a grid; `subTabs` shows one at a time. */
  layout: 'onePage' | 'subTabs';
  /** The panel arrangement, when the page supports arranging. */
  panels?: PanelLayout[];
  /** Applies an arrangement change. */
  onPanelsChange?: (next: PanelLayout[]) => void;
}

/**
 * Presents a page's sections according to the layout preference.
 *
 * @param props The sections, the chosen layout, and the panel arrangement.
 */
export function SectionContainer({
  sections,
  layout,
  panels,
  onPanelsChange,
}: SectionContainerProps): React.JSX.Element {
  const [active, setActive] = useState(sections[0]?.id ?? '');

  // Ordered by the arrangement where there is one, so the sub-tab order is the
  // same order the grid uses. Hidden panels are hidden in both.
  const arranged =
    panels === undefined
      ? sections
      : panels
          .filter((panel) => !panel.hidden)
          .flatMap((panel) => sections.filter((section) => section.id === panel.id));

  if (layout === 'subTabs') {
    const current = arranged.find((section) => section.id === active) ?? arranged[0];

    // One menu, acting on the selected tab. Width and height are omitted:
    // one section fills the pane, so they would do nothing, and a control that
    // does nothing is worse than no control. Order still matters here — it is
    // the tab order — and so does hiding.
    const index = current === undefined ? -1 : arranged.findIndex((s) => s.id === current.id);

    return (
      <div className="flex flex-col gap-3">
        <div
          role="tablist"
          aria-label="Sections"
          className="flex items-center gap-1 border-b border-edge"
        >
          {arranged.map((section) => {
            const selected = section.id === current?.id;
            return (
              <button
                key={section.id}
                type="button"
                role="tab"
                aria-selected={selected}
                onClick={() => {
                  setActive(section.id);
                }}
                className={`-mb-px border-b-2 px-3 py-2 text-sm ${
                  selected
                    ? 'border-accent text-text'
                    : 'border-transparent text-text-muted hover:text-text'
                }`}
              >
                {section.title}
              </button>
            );
          })}

          {current !== undefined && panels !== undefined && onPanelsChange !== undefined && (
            <div className="ml-auto pb-1">
              <PanelMenu
                title={current.title}
                span={12}
                height="normal"
                canMoveUp={index > 0}
                canMoveDown={index >= 0 && index < arranged.length - 1}
                showSize={false}
                onSpan={() => {}}
                onHeight={() => {}}
                onMove={(delta) => {
                  onPanelsChange(moveVisibleById(panels, current.id, delta));
                }}
                onHide={() => {
                  const remaining = arranged.filter((section) => section.id !== current.id);
                  // Move off the tab about to disappear, or the panel would be
                  // hidden while still selected and the pane would go blank.
                  setActive(remaining[0]?.id ?? '');
                  onPanelsChange(updatePanel(panels, current.id, { hidden: true }));
                }}
              />
            </div>
          )}
        </div>

        {current !== undefined && (
          <div role="tabpanel" aria-label={current.title}>
            {current.content}
          </div>
        )}
      </div>
    );
  }

  if (panels !== undefined && onPanelsChange !== undefined) {
    return <PanelGrid sections={sections} layout={panels} onLayoutChange={onPanelsChange} />;
  }

  return (
    <div className="flex flex-col gap-2">
      {arranged.map((section) => (
        <Collapsible key={section.id} section={section} defaultOpen />
      ))}
    </div>
  );
}
