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
