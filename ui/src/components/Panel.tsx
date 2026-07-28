/**
 * One page section, and the frame it is drawn in.
 *
 * Separate from `Section.tsx` because `SectionContainer` delegates to
 * `PanelGrid`, and `PanelGrid` renders this. Leaving both here would make those
 * two modules import each other, and a cycle between component modules resolves
 * differently depending on which side a bundler evaluates first.
 */

import { useId, useState, type ReactNode } from 'react';

import { PanelMenu, type PanelControls } from './PanelMenu';

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
  /** The arrangement menu's behaviour, when the section can be arranged. */
  controls?: PanelControls;
  /** The drag grip, when reordering is available. */
  grip?: ReactNode;
}

/**
 * Renders one collapsible section.
 *
 * The summary stays visible when collapsed. A section that hid its headline
 * figure when shut would make collapsing cost information rather than space,
 * which is the opposite of the point.
 *
 * The header is a flex container rather than a single button because a grip and
 * a menu button cannot nest inside one: nested buttons are invalid HTML and
 * behave inconsistently across browsers. The collapse target is the middle
 * child, so clicking the title still toggles and clicking the controls does not.
 */
export function Collapsible({
  section,
  defaultOpen,
  controls,
  grip,
}: CollapsibleProps): React.JSX.Element {
  const [open, setOpen] = useState(defaultOpen);
  const contentId = useId();

  return (
    <section className="overflow-hidden rounded-xl border border-edge bg-surface-raised">
      <h3 className="flex items-center gap-1 pr-1.5">
        {grip}

        <button
          type="button"
          aria-expanded={open}
          aria-controls={contentId}
          onClick={() => {
            setOpen((current) => !current);
          }}
          className="flex min-w-0 flex-1 items-center gap-2 px-2 py-2.5 text-left hover:bg-white/[0.03]"
        >
          <span aria-hidden="true" className="text-xs text-accent">
            {open ? '▾' : '▸'}
          </span>
          <span className="truncate text-sm font-semibold">{section.title}</span>
          {section.summary !== undefined && (
            <span className="ml-auto truncate font-mono text-sm text-neutral-300">
              {section.summary}
            </span>
          )}
        </button>

        {controls !== undefined && <PanelMenu title={section.title} {...controls} />}
      </h3>

      {open && (
        <div id={contentId} className="border-t border-edge/70 px-4 py-3">
          {section.content}
        </div>
      )}
    </section>
  );
}
