/**
 * The four dark themes, and how one gets applied.
 *
 * A theme is nothing but a `data-theme` value on `<html>`. Every colour lives
 * in `index.css` as a token, and each theme is a block that overrides those
 * tokens; no colour is computed here, held in state, or passed to a component.
 * That is what keeps the switch to one DOM attribute — and it is what lets the
 * boot script in `public/theme-boot.js` apply the stored theme before React
 * exists, which is the difference between a themed app and an app that flashes
 * the default theme on every launch.
 *
 * All four are dark. This is a system utility that sits beside a terminal, and
 * a light theme would be a different design rather than another set of values
 * for this one — the surfaces, the chart palette and the meter ramps are all
 * built for a dark background.
 */

/** Which theme the interface is wearing. */
export type Theme = 'midnight' | 'carbon' | 'contrast' | 'terminal';

/** The attribute that carries the theme, on `<html>`. */
export const THEME_ATTRIBUTE = 'data-theme';

/**
 * Every theme, in the order the settings row offers them.
 *
 * The descriptions say what each one is *for* rather than what it looks like,
 * because the swatch beside it already shows what it looks like.
 */
export const THEMES = [
  {
    value: 'midnight',
    label: 'Midnight',
    description: 'Cool near-black with a blue cast. What osstat has always looked like.',
  },
  {
    value: 'carbon',
    label: 'Carbon',
    description: 'Warm near-black, amber accent. Easier on the eyes late at night.',
  },
  {
    value: 'contrast',
    label: 'Contrast',
    description: 'Near-pure black, and every tier of text brighter. For a lit room.',
  },
  {
    value: 'terminal',
    label: 'Terminal',
    description: 'Green phosphor, for sitting alongside a terminal that looks the same.',
  },
] as const satisfies readonly { value: Theme; label: string; description: string }[];

/** What a fresh install wears. */
export const DEFAULT_THEME: Theme = 'midnight';

/**
 * Whether `value` names a theme.
 *
 * @param value Anything, typically off storage.
 * @returns Whether it is one of the four.
 */
export function isTheme(value: unknown): value is Theme {
  return THEMES.some((theme) => theme.value === value);
}

/**
 * Puts a theme on the document.
 *
 * Writes the attribute unconditionally rather than diffing against the current
 * value: the boot script has usually set it already, and re-setting it to the
 * same string costs nothing while a mismatch — React's idea of the theme
 * disagreeing with the document's — would be invisible until someone changed
 * an unrelated setting.
 *
 * @param theme The theme to apply.
 * @param root The element to mark, defaulting to `<html>`.
 */
export function applyTheme(theme: Theme, root: HTMLElement = document.documentElement): void {
  root.setAttribute(THEME_ATTRIBUTE, theme);
}
