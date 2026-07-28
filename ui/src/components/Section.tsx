/**
 * A page section, and the container that decides how sections are presented.
 *
 * The two view settings are independent, and this is half of why that is cheap:
 * a section knows nothing about whether it is stacked or tabbed. The container
 * renders the same components either way.
 */

import { useId, useState, type ReactNode } from 'react';

/** One section of a page. */
export interface SectionSpec {
  /** Stable identifier, used for the sub-tab selection. */
  id: string;
  /** Heading shown on the section and on its tab. */
  title: string;
  /** The current headline figure, shown on the collapsed header. */
  summary?: ReactNode;
  /** The section body. */
  content: ReactNode;
}

/** A collapsible section. */
interface CollapsibleProps {
  /** The section to render. */
  section: SectionSpec;
  /** Whether it starts open. */
  defaultOpen: boolean;
}

/**
 * Renders one collapsible section.
 *
 * The summary stays visible when collapsed. A section that hid its headline
 * figure when shut would make collapsing cost information rather than space,
 * which is the opposite of the point.
 */
function Collapsible({ section, defaultOpen }: CollapsibleProps): React.JSX.Element {
  const [open, setOpen] = useState(defaultOpen);
  const contentId = useId();

  return (
    <section className="overflow-hidden rounded-xl border border-edge bg-surface-raised">
      <h3>
        <button
          type="button"
          aria-expanded={open}
          aria-controls={contentId}
          onClick={() => {
            setOpen((current) => !current);
          }}
          className="flex w-full items-center gap-2 px-4 py-2.5 text-left hover:bg-white/[0.03]"
        >
          <span aria-hidden="true" className="text-xs text-accent">
            {open ? '▾' : '▸'}
          </span>
          <span className="text-sm font-semibold">{section.title}</span>
          {section.summary !== undefined && (
            <span className="ml-auto font-mono text-sm text-neutral-300">{section.summary}</span>
          )}
        </button>
      </h3>

      {open && (
        <div id={contentId} className="border-t border-edge/70 px-4 py-3">
          {section.content}
        </div>
      )}
    </section>
  );
}

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
