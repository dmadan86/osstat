/**
 * The guard on the colour tokens.
 *
 * These are not tests of a component; they are tests of a rule that a
 * component can break silently. A stray `text-neutral-<n>` renders as the same
 * grey under every theme, and nothing about it looks wrong on the theme it was
 * written against — it only looks wrong on one of the other three, on a screen
 * whoever added it had no reason to open. So the rule is checked here instead
 * of being noticed later.
 *
 * The shade numbers are written `<n>` throughout this file on purpose: the
 * scan below reads every file under `ui/src`, this one included, and a rule
 * that has to exempt its own statement of itself is a weaker rule.
 */

import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

import { SERIES } from './charts/palette';
import { THEMES, type Theme } from './lib/theme';

const SOURCE_ROOT = join(import.meta.dirname, '.');
const STYLESHEET = join(SOURCE_ROOT, 'index.css');

/** Every file under `ui/src`, at any depth, as a repo-relative path. */
function sourceFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    return entry.isDirectory() ? sourceFiles(path) : [path];
  });
}

/** A file and the line that broke a rule, worded so the failure names it. */
function offendingLines(pattern: RegExp): string[] {
  return sourceFiles(SOURCE_ROOT).flatMap((path) => {
    // Not `utf8`-and-split alone: `advisor.ts` carries a deliberate NUL as a
    // composite-key separator, which makes tools that sniff for binary skip
    // the file. Reading it explicitly is the difference between checking it
    // and quietly exempting it.
    const lines = readFileSync(path, 'utf8').split('\n');
    return lines.flatMap((line, index) =>
      pattern.test(line) ? [`${path.replaceAll('\\', '/')}:${index + 1}: ${line.trim()}`] : []
    );
  });
}

describe('the colour tokens', () => {
  it('leaves no stock Tailwind palette shade anywhere in the front end', () => {
    // The full stock palette, not just the greys that were migrated. Fixing
    // those and then letting somebody reach for `slate-<n>` next week would
    // rebuild exactly the problem this replaced. The state colours —
    // red, amber, emerald, sky — are deliberately absent from this list: they
    // mean one specific thing, they are not surface-dependent, and they always
    // ship with an icon or a word beside them.
    const shades = ['neutral', 'gray', 'grey', 'zinc', 'stone', 'slate'].join('|');
    const offenders = offendingLines(new RegExp(`\\b(?:${shades})-[0-9]{2,3}\\b`));

    expect(offenders).toEqual([]);
  });

  it('spends every themeable token it defines', () => {
    // A token nothing uses is a token nobody maintains: it drifts out of step
    // with the rest of a theme and is discovered when somebody finally uses it.
    const stylesheet = readFileSync(STYLESHEET, 'utf8');
    const defined = [...stylesheet.matchAll(/^\s{2}--color-([a-z-]+):/gm)].map((match) => match[1]);
    const front = sourceFiles(SOURCE_ROOT)
      .filter((path) => path.endsWith('.tsx') || path.endsWith('.ts'))
      .map((path) => readFileSync(path, 'utf8'))
      .join('\n');

    expect(defined.length).toBeGreaterThan(0);

    const unused = defined.filter(
      (token) =>
        // As a utility (`text-text-muted`, `bg-chart-up`, `border-edge`) or by
        // variable, which is how `shadow-[...]` and the boot script reach one.
        !new RegExp(`[a-z:\\[]-?${token}\\b|--color-${token}\\b`).test(front) &&
        !new RegExp(`--color-${token}\\b`).test(stylesheet.split('@theme')[1] ?? '')
    );

    expect(unused).toEqual([]);
  });

  it('keeps the network legend swatches equal to the lines they label', () => {
    // The chart is drawn by ECharts from `palette.ts`; the legend beside it is
    // drawn by CSS from these tokens. Nothing in the type system connects the
    // two, so a change to one silently makes the legend describe a colour that
    // is no longer on screen.
    const stylesheet = readFileSync(STYLESHEET, 'utf8');

    expect(stylesheet).toContain(`--color-chart-down: ${SERIES[0]};`);
    expect(stylesheet).toContain(`--color-chart-up: ${SERIES[1]};`);
  });
});

/** The `--color-*` declarations inside one theme's block, by token name. */
function tokensOf(theme: Theme): Map<string, string> {
  const stylesheet = readFileSync(STYLESHEET, 'utf8');
  const block = new RegExp(`\\[data-theme='${theme}'\\]\\s*\\{([^}]*)\\}`).exec(stylesheet)?.[1];

  if (block === undefined) throw new Error(`index.css defines no block for '${theme}'`);

  return new Map(
    [...block.matchAll(/--color-([a-z-]+):\s*([^;]+);/g)].map((match) => [
      match[1] ?? '',
      (match[2] ?? '').trim(),
    ])
  );
}

/** The `--color-*` names declared before the theme blocks, i.e. in `@theme`. */
function baseTokenNames(): string[] {
  const base = readFileSync(STYLESHEET, 'utf8').split('}')[0] ?? '';
  return [...base.matchAll(/--color-([a-z-]+):/g)].map((match) => match[1] ?? '');
}

/**
 * How light one theme's token is, on the `oklch` 0..1 scale.
 *
 * Throws rather than returning a default when the token or the colour space is
 * not what it should be: a missing token silently reading as `0` would let a
 * theme pass the ordering checks below by being absent.
 */
function lightness(theme: Theme, token: string): number {
  const value = tokensOf(theme).get(token);

  if (value === undefined) throw new Error(`'${theme}' declares no --color-${token}`);

  const measured = /oklch\(\s*([0-9.]+)/.exec(value)?.[1];

  if (measured === undefined) throw new Error(`--color-${token} on '${theme}' is not oklch`);

  return Number(measured);
}

describe('the four themes', () => {
  it('each override the same complete set of tokens', () => {
    // A theme that forgot `--color-text-faint` would inherit midnight's, and
    // grey-blue timestamps on a green surface is exactly the kind of thing
    // that survives review because nobody scrolled to a timestamp.
    const expected = [...tokensOf('midnight').keys()].sort();

    expect(expected.length).toBeGreaterThan(0);

    for (const theme of THEMES) {
      expect({ [theme.value]: [...tokensOf(theme.value).keys()].sort() }).toEqual({
        [theme.value]: expected,
      });
    }
  });

  it('themes every token except the chart pair', () => {
    // Keeps the two sets from drifting apart in either direction: a token
    // added to `@theme` and forgotten here would be frozen at midnight's value
    // under all four themes.
    expect(new Set(tokensOf('midnight').keys())).toEqual(
      new Set(baseTokenNames().filter((token) => !token.startsWith('chart-')))
    );
  });

  it('leaves midnight identical to the values outside the theme blocks', () => {
    // `@theme` is what an unthemed document wears, so if these disagreed the
    // very first frame of a first run would be a colour no theme defines.
    const base = readFileSync(STYLESHEET, 'utf8').split('}')[0] ?? '';

    for (const [token, value] of tokensOf('midnight')) {
      expect(base).toContain(`--color-${token}: ${value};`);
    }
  });

  it('is dark on all four, which the settings row promises outright', () => {
    for (const theme of THEMES) {
      expect({ [theme.value]: lightness(theme.value, 'surface') < 0.25 }).toEqual({
        [theme.value]: true,
      });
    }
  });

  it('makes contrast actually higher-contrast, tier by tier', () => {
    // The claim the theme is named for. Before the text ramp became a token
    // this could not have been true at all: every tier was a fixed grey, so
    // "high contrast" would have moved the panels and left every word on them
    // exactly as legible as before.
    for (const tier of ['text', 'text-muted', 'text-faint']) {
      expect({
        tier,
        lighter: lightness('contrast', tier) > lightness('midnight', tier),
      }).toEqual({ tier, lighter: true });
    }

    // Its surface is darker as well -- brighter text on the same background
    // would be half the job.
    expect(lightness('contrast', 'surface')).toBeLessThan(lightness('midnight', 'surface'));

    // The tier that gains most is the one that was hardest to read: faint text
    // on contrast outruns *muted* text on midnight.
    expect(lightness('contrast', 'text-faint')).toBeGreaterThan(
      lightness('midnight', 'text-muted')
    );
  });

  it('keeps each ramp ordered, so a tier never outshines the one above it', () => {
    for (const theme of THEMES) {
      const ramp = ['text', 'text-muted', 'text-faint'].map((tier) => lightness(theme.value, tier));

      expect({ theme: theme.value, ramp }).toEqual({
        theme: theme.value,
        ramp: [...ramp].sort((left, right) => right - left),
      });

      // And the faintest tier stays clear of the surface it is drawn on.
      expect({
        theme: theme.value,
        readable: lightness(theme.value, 'text-faint') - lightness(theme.value, 'surface') > 0.2,
      }).toEqual({ theme: theme.value, readable: true });
    }
  });
});
