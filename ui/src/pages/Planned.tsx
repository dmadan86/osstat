/**
 * The page shown for a capability that does not exist yet.
 *
 * It says what is coming and which milestone builds it, rather than rendering
 * an empty table that looks like a broken version of the real thing. The README
 * makes a point of not implying a finished app, and a page is where that
 * promise is either kept or quietly broken.
 */

import type { NavItem } from '../routes';
import { PLANNED } from '../routes';

/** What the placeholder needs. */
export interface PlannedProps {
  /** The navigation entry for the unbuilt page. */
  item: NavItem;
}

/**
 * Renders the placeholder for an unbuilt capability.
 *
 * @param props The navigation entry describing the page.
 */
export function Planned({ item }: PlannedProps): React.JSX.Element {
  const details = PLANNED[item.route];

  return (
    <div className="flex max-w-xl flex-col items-start gap-3 rounded-xl border border-dashed border-edge bg-surface-raised px-6 py-8">
      <span className="rounded-full border border-edge px-2 py-0.5 font-mono text-xs text-neutral-400">
        {item.milestone}
      </span>

      <h2 className="text-lg font-semibold">{details?.title ?? item.label}</h2>

      <p className="text-sm leading-relaxed text-neutral-400">
        {details?.summary ?? 'Not built yet.'}
      </p>

      <p className="text-xs text-neutral-600">
        Milestones are built in order, each gated on the previous one passing. See ROADMAP.md.
      </p>
    </div>
  );
}
