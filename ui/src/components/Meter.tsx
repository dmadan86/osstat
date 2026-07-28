/**
 * A single ratio against a limit.
 *
 * A meter, not a pie and not a one-bar bar chart: the question "how full is
 * this disk" is one number against a known maximum, and a track filled to a
 * fraction answers it in less space than any chart would.
 */

import { diskStatus, INK, SEQUENTIAL, STATUS } from '../charts/palette';
import { formatFraction } from '../lib/format';

/** Everything a meter needs. */
export interface MeterProps {
  /** Occupancy in `0..=1`. */
  fraction: number;
  /** What is being measured. */
  label: string;
  /** The figure shown beside the label, e.g. `612 GB of 931 GB`. */
  detail?: string;
  /** Apply the fullness status treatment. Off for meters where full is normal. */
  warnWhenFull?: boolean;
}

/** Wording for each status, so colour is never the only signal. */
const STATUS_TEXT = {
  serious: 'Low space',
  critical: 'Almost full',
} as const;

/**
 * Renders a labelled meter.
 *
 * When a volume crosses a fullness threshold the bar changes colour **and**
 * gains an icon and the words "Low space". A status colour never carries
 * meaning on its own — a bar that silently turned red would say nothing at all
 * to a reader who cannot distinguish it from the normal one.
 *
 * @param props The fraction, label and optional detail line.
 */
export function Meter({
  fraction,
  label,
  detail,
  warnWhenFull = false,
}: MeterProps): React.JSX.Element {
  const safe = Number.isFinite(fraction) ? Math.min(1, Math.max(0, fraction)) : 0;
  const status = warnWhenFull ? diskStatus(safe) : null;
  const fill = status === null ? SEQUENTIAL[3] : STATUS[status];

  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-baseline justify-between gap-3 text-sm">
        <span className="truncate">{label}</span>
        <span data-selectable className="font-mono text-xs text-neutral-300">
          {formatFraction(safe)}
        </span>
      </div>

      <div
        role="meter"
        aria-valuenow={Math.round(safe * 100)}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label={label}
        className="h-1.5 w-full overflow-hidden rounded-full"
        style={{ backgroundColor: INK.gridline }}
      >
        <div
          className="h-full rounded-full transition-[width] duration-500"
          style={{ width: `${safe * 100}%`, backgroundColor: fill }}
        />
      </div>

      <div className="flex items-center justify-between gap-3">
        {detail !== undefined && (
          <span data-selectable className="text-xs text-neutral-500">
            {detail}
          </span>
        )}
        {status !== null && (
          <span
            className="ml-auto flex items-center gap-1 text-xs font-medium"
            style={{ color: STATUS[status] }}
          >
            <span aria-hidden="true">▲</span>
            {STATUS_TEXT[status]}
          </span>
        )}
      </div>
    </div>
  );
}
