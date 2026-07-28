/**
 * A page section, and the container that decides how sections are presented.
 *
 * The two view settings are independent, and this is half of why that is cheap:
 * a section knows nothing about whether it is stacked or tabbed. The container
 * renders the same components either way.
 */

import { useState } from 'react';

import { Collapsible, type SectionSpec } from './Panel';

export type { SectionSpec } from './Panel';

/** How a page presents its sections. */
export interface SectionContainerProps {
  /** The sections, in order. */
  sections: SectionSpec[];
  /** `onePage` stacks them collapsibly; `subTabs` shows one at a time. */
  layout: 'onePage' | 'subTabs';
}

/**
 * Presents a page's sections according to the layout preference.
 *
 * @param props The sections and the chosen layout.
 */
export function SectionContainer({ sections, layout }: SectionContainerProps): React.JSX.Element {
  const [active, setActive] = useState(sections[0]?.id ?? '');

  if (layout === 'subTabs') {
    const current = sections.find((section) => section.id === active) ?? sections[0];

    return (
      <div className="flex flex-col gap-3">
        <div role="tablist" aria-label="Sections" className="flex gap-1 border-b border-edge">
          {sections.map((section) => {
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
                    ? 'border-accent text-neutral-100'
                    : 'border-transparent text-neutral-400 hover:text-neutral-200'
                }`}
              >
                {section.title}
              </button>
            );
          })}
        </div>

        {current !== undefined && (
          <div role="tabpanel" aria-label={current.title}>
            {current.content}
          </div>
        )}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-2">
      {sections.map((section) => (
        <Collapsible key={section.id} section={section} defaultOpen />
      ))}
    </div>
  );
}
