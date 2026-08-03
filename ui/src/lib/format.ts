/**
 * Display formatting.
 *
 * Kept in one place because these decisions have to agree with the backend's:
 * the sampler decides a process "changed" by comparing CPU at a tenth of a
 * percent and memory at a mebibyte, so rendering CPU to three decimals would
 * show a value that visibly disagrees with how often it updates.
 */

const KIB = 1024;
const UNITS = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'] as const;

/**
 * Formats a byte count with a binary-scaled unit.
 *
 * Precision falls as the unit grows: `847 MB` and `1.42 GB` both carry about
 * three significant figures, which is as much as anyone reads off a dashboard.
 *
 * @param bytes A non-negative byte count.
 * @returns A string such as `1.42 GB`, or `—` if the value is not a number.
 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '—';
  if (bytes === 0) return '0 B';

  let value = bytes;
  let unit = 0;
  while (value >= KIB && unit < UNITS.length - 1) {
    value /= KIB;
    unit += 1;
  }

  const digits = unit === 0 ? 0 : value < 10 ? 2 : value < 100 ? 1 : 0;
  return `${value.toFixed(digits)} ${UNITS[unit]}`;
}

/**
 * Formats a throughput.
 *
 * @param bytesPerSecond Bytes per second.
 * @returns A string such as `1.42 GB/s`.
 */
export function formatRate(bytesPerSecond: number): string {
  if (!Number.isFinite(bytesPerSecond) || bytesPerSecond <= 0) return '—';
  return `${formatBytes(bytesPerSecond)}/s`;
}

/**
 * Formats a percentage at the precision the backend diffs on.
 *
 * @param percent A value in `0..=100` (or above, for multi-core CPU figures).
 * @returns A string such as `12.3%`.
 */
export function formatPercent(percent: number): string {
  if (!Number.isFinite(percent)) return '—';
  return `${percent.toFixed(1)}%`;
}

/**
 * Formats a fraction as a whole-number percentage.
 *
 * @param fraction A value in `0..=1`.
 * @returns A string such as `66%`.
 */
export function formatFraction(fraction: number): string {
  if (!Number.isFinite(fraction)) return '—';
  return `${Math.round(fraction * 100)}%`;
}

/**
 * Formats a whole count with thousands separators.
 *
 * Grouped by hand rather than through `toLocaleString`, so the string does not
 * change with the machine's locale — `1,048 of 4,096 tokens` is asserted on in
 * tests and read off a meter, and a separator that varied by host would make
 * both unstable.
 *
 * @param value A count; fractions are rounded.
 * @returns A string such as `4,096`.
 */
export function formatCount(value: number): string {
  if (!Number.isFinite(value)) return '—';
  return Math.round(value)
    .toString()
    .replace(/\B(?=(\d{3})+(?!\d))/g, ',');
}

/**
 * Formats an elapsed time in coarse, human units.
 *
 * Deliberately imprecise above an hour: nobody reads uptime to the second, and
 * a ticking seconds field is a distraction on a page that redraws every tick.
 *
 * @param seconds Elapsed seconds.
 * @returns A string such as `2 d 4 h`, `19 m` or `44 s`.
 */
export function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return '—';

  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);

  if (days > 0) return `${days} d ${hours} h`;
  if (hours > 0) return `${hours} h ${minutes} m`;
  if (minutes > 0) return `${minutes} m`;
  return `${Math.floor(seconds)} s`;
}

/**
 * Formats a wall-clock time for a chart axis.
 *
 * @param epochMs Milliseconds since the Unix epoch.
 * @returns A `HH:MM:SS` string in local time.
 */
export function formatClock(epochMs: number): string {
  if (!Number.isFinite(epochMs)) return '—';
  return new Date(epochMs).toLocaleTimeString(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  });
}

/**
 * Formats a wall-clock time of day, without seconds.
 *
 * Distinct from {@link formatClock}, which is for a chart axis and carries
 * seconds because samples arrive faster than a minute. A message in a
 * transcript is stamped once, and the second it landed on is noise beside the
 * text it is labelling.
 *
 * 24-hour for the same reason `formatClock` is: the hour a reply arrived is
 * read against the one above it, and `11:58 PM` beside `12:03 AM` reads as
 * going backwards.
 *
 * @param epochMs Milliseconds since the Unix epoch.
 * @returns A `HH:MM` string in local time, or `—` if the value is not a number.
 */
export function formatTimeOfDay(epochMs: number): string {
  if (!Number.isFinite(epochMs)) return '—';
  return new Date(epochMs).toLocaleTimeString(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  });
}

/**
 * Formats a frequency in megahertz.
 *
 * @param megahertz Clock speed in MHz; zero means the platform did not report one.
 * @returns A string such as `4.50 GHz`, or `—` when unreported.
 */
export function formatFrequency(megahertz: number): string {
  if (!Number.isFinite(megahertz) || megahertz <= 0) return '—';
  if (megahertz >= 1000) return `${(megahertz / 1000).toFixed(2)} GHz`;
  return `${Math.round(megahertz)} MHz`;
}
