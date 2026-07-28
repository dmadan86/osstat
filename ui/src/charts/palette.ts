/**
 * The chart palette, chosen by measurement rather than by eye.
 *
 * Every value below was run through the data-visualisation validator against
 * osstat's actual chart surface — the raised panel, `--color-surface-raised`,
 * `oklch(0.24 0.024 265)` = `#1a1f2b` — in dark mode. Results:
 *
 * - The eight reference categorical slots pass every adjacent-pair gate on this
 *   surface: worst adjacent colour-vision-deficiency ΔE 8.4, worst adjacent
 *   normal-vision ΔE 19.3, all at or above 3:1 contrast.
 * - The first three slots additionally pass under `--pairs all` (CVD ΔE 9.4,
 *   normal-vision ΔE 20.9). No chart here needs more than three series, so
 *   {@link SERIES} stops at three and no chart may exceed it.
 *
 * One finding changed existing code. osstat's UI accent,
 * `oklch(0.72 0.15 232)` = `#00b3f2`, sits at lightness 0.721 — outside the
 * 0.48–0.67 band a data mark must occupy — so it is **not** used for any chart
 * series. It remains the accent for selection, focus and active navigation,
 * where the band does not apply. Charts lead with {@link SERIES}[0] instead.
 *
 * If you change a colour here, re-run the validator. Do not eyeball it.
 */

/**
 * Categorical series colours, assigned in fixed order and never cycled.
 *
 * Order is the colour-vision-deficiency safety mechanism, not decoration. A
 * fourth series is not a fourth colour: fold it into the existing three, facet
 * the chart, or reconsider the encoding.
 */
export const SERIES = ['#3987e5', '#d95926', '#199e70'] as const;

/**
 * The single hue used for magnitude, light to dark.
 *
 * Anything that is "more is bigger" rather than "these are different things"
 * uses this: the per-core heatmap, meters, the CPU history area. On a dark
 * surface an ordinal step must go no darker than `#184f95` to hold 2:1 against
 * the background, which is where this ramp stops.
 */
export const SEQUENTIAL = [
  '#cde2fb',
  '#9ec5f4',
  '#6da7ec',
  '#3987e5',
  '#256abf',
  '#184f95',
] as const;

/**
 * The same ramp, ordered for a filled mark on a dark surface.
 *
 * A sequential ramp's low end should recede toward the surface it sits on, and
 * osstat's surface is dark. Used light-to-dark here, an idle CPU core would
 * glare almost white while a saturated one sank into the background — the
 * encoding inverted, and the brightest thing on screen would be the part doing
 * nothing.
 *
 * Same six steps, same validation; only the direction changes, and it changes
 * because of what they are drawn on.
 */
export const SEQUENTIAL_ON_DARK = [...SEQUENTIAL].reverse();

/**
 * Reserved state colours.
 *
 * Never reused as a series colour, and never the only carrier of meaning: a
 * status colour always ships with an icon and a label. In this app they apply
 * to exactly one thing — a disk crossing a fullness threshold.
 */
export const STATUS = {
  good: '#0ca30c',
  warning: '#fab219',
  serious: '#ec835a',
  critical: '#d03b3b',
} as const;

/** Chart chrome. Text wears these, never a series colour. */
export const INK = {
  /** The panel charts are drawn on; also the gap colour between fills. */
  surface: '#1a1f2b',
  primary: '#e4e8ef',
  secondary: '#9aa4b8',
  muted: '#6b7688',
  gridline: '#252b39',
  axis: '#2f3543',
} as const;

/** Fill opacity for an area under a line. */
export const AREA_OPACITY = 0.18;

/**
 * How full a disk must be before it stops being reported as merely a number.
 *
 * Paired with an icon and the words "Low space" wherever it is used — a bar
 * that silently turns red is a status colour carrying meaning alone.
 */
export const DISK_THRESHOLDS = {
  /** At or above this fraction, the volume is worth flagging. */
  serious: 0.9,
  /** At or above this fraction, the volume is nearly unusable. */
  critical: 0.97,
} as const;

/**
 * Classifies a disk's occupancy for display.
 *
 * @param fraction Occupancy in `0..=1`.
 * @returns The status key, or `null` when the volume is unremarkable.
 */
export function diskStatus(fraction: number): 'serious' | 'critical' | null {
  if (fraction >= DISK_THRESHOLDS.critical) return 'critical';
  if (fraction >= DISK_THRESHOLDS.serious) return 'serious';
  return null;
}
