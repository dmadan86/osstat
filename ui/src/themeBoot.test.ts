/**
 * The pre-paint theme boot script.
 *
 * The bug this file exists to catch is the one this kind of feature always
 * ships with: the theme is stored, the theme is applied, everything looks
 * right in a screenshot — and every launch opens on the default theme for a
 * frame before correcting itself. Nothing in a component test can see that,
 * because by the time a component renders the moment has passed.
 *
 * So these tests do not render anything. They take `public/theme-boot.js` as
 * the browser gets it, run it against a seeded store with no React in the
 * process at all, and check the attribute is on the document when it returns.
 * A theme applied by an effect cannot pass this file.
 */

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { beforeEach, describe, expect, it } from 'vitest';

import { STORAGE_KEY } from './lib/preferences';
import { THEMES } from './lib/theme';

const UI_ROOT = join(import.meta.dirname, '..');
const BOOT_SCRIPT = readFileSync(join(UI_ROOT, 'public', 'theme-boot.js'), 'utf8');
const INDEX_HTML = readFileSync(join(UI_ROOT, 'index.html'), 'utf8');

/**
 * Runs the boot script exactly as a `<script>` tag would.
 *
 * Deliberately not an import: the file is copied into the bundle rather than
 * compiled, so evaluating the bytes on disk is what tests the thing that
 * actually ships.
 */
function runBootScript(): void {
  new Function(BOOT_SCRIPT)();
}

/** What the boot script left on the document, if anything. */
function appliedTheme(): string | null {
  return document.documentElement.getAttribute('data-theme');
}

describe('the theme boot script', () => {
  beforeEach(() => {
    window.localStorage.clear();
    document.documentElement.removeAttribute('data-theme');
  });

  it('applies each stored theme synchronously, before anything renders', () => {
    for (const theme of THEMES) {
      document.documentElement.removeAttribute('data-theme');
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify({ theme: theme.value }));

      runBootScript();

      // Asserted immediately after the call returns, with no `await`, no timer
      // and no flush. Anything that reached for the DOM later -- an effect, a
      // microtask, `requestAnimationFrame` -- would fail here, which is the
      // only property that distinguishes this from theming in React.
      expect(appliedTheme()).toBe(theme.value);
    }
  });

  it('reads the key preferences are actually written under', () => {
    // The script cannot import `STORAGE_KEY`, so it repeats the string. If the
    // two ever drift, the boot script quietly finds nothing and every launch
    // opens on the default theme -- with the stored preference intact and the
    // settings page showing the right choice, which is the worst version of
    // this bug to debug.
    expect(BOOT_SCRIPT).toContain(STORAGE_KEY);
  });

  it('accepts exactly the themes that exist', () => {
    // The other repeated literal. A theme added to `theme.ts` but not here
    // would be selectable, would persist, and would fail to survive a restart.
    for (const theme of THEMES) {
      expect(BOOT_SCRIPT).toContain(`'${theme.value}'`);
    }

    window.localStorage.setItem(STORAGE_KEY, JSON.stringify({ theme: 'solarized' }));
    runBootScript();

    expect(appliedTheme()).toBeNull();
  });

  it('leaves the document alone when there is nothing to apply', () => {
    for (const stored of [null, '{not json', '{}', '[]', 'null', '{"theme":null}']) {
      document.documentElement.removeAttribute('data-theme');
      if (stored === null) window.localStorage.removeItem(STORAGE_KEY);
      else window.localStorage.setItem(STORAGE_KEY, stored);

      // The assertion is as much that this does not throw. It runs before the
      // error handlers in `main.tsx` are installed, so an exception here is an
      // app that opens to a blank window with nothing written anywhere.
      expect(runBootScript).not.toThrow();
      expect(appliedTheme()).toBeNull();
    }
  });

  it('survives storage being unavailable entirely', () => {
    // A webview can refuse `localStorage` outright, and reading it throws
    // rather than returning null.
    const original = Object.getOwnPropertyDescriptor(window, 'localStorage');
    Object.defineProperty(window, 'localStorage', {
      configurable: true,
      get() {
        throw new Error('storage is disabled');
      },
    });

    try {
      expect(runBootScript).not.toThrow();
      expect(appliedTheme()).toBeNull();
    } finally {
      if (original !== undefined) Object.defineProperty(window, 'localStorage', original);
    }
  });
});

describe('the boot script tag', () => {
  it('is loaded in a way that runs before the first paint', () => {
    const tag = /<script\b[^>]*\btheme-boot\.js[^>]*>/.exec(INDEX_HTML)?.[0];

    expect(tag).toBeDefined();
    // Each of these would push execution past first paint and hand back the
    // flash of the default theme, while leaving every other test in this file
    // passing -- the script would still work, just too late to matter.
    expect(tag).not.toMatch(/\bdefer\b/);
    expect(tag).not.toMatch(/\basync\b/);
    expect(tag).not.toMatch(/type\s*=\s*["']module["']/);
  });

  it('runs before the module that mounts React', () => {
    // Order in the document is the guarantee. A blocking script placed after
    // the app's entry point is not a boot script.
    expect(INDEX_HTML.indexOf('theme-boot.js')).toBeLessThan(INDEX_HTML.indexOf('/src/main.tsx'));
  });
});
