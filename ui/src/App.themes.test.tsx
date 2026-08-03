/**
 * Choosing a theme, and finding it still chosen after a restart.
 *
 * The pieces are tested apart elsewhere — the tokens in `tokens.test.ts`, the
 * boot script in `themeBoot.test.ts`, the settings row in
 * `Settings.themes.test.tsx`. This file is the seam between them, because
 * every way this feature breaks in practice is a seam: the click applies a
 * theme it never stores, or stores one under a key nothing reads back, or
 * stores it correctly and opens on the default anyway.
 */

import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { App } from './App';
import type { SystemDescription } from './bindings/SystemDescription';
import { STORAGE_KEY } from './lib/preferences';
import { THEMES } from './lib/theme';

const invoke = vi.hoisted(() => vi.fn());
const listen = vi.hoisted(() => vi.fn());

vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen }));

vi.mock('./components/Chart', () => ({
  Chart: ({ label }: { label: string }) => <div role="img" aria-label={label} />,
}));

const BOOT_SCRIPT = readFileSync(
  join(import.meta.dirname, '..', 'public', 'theme-boot.js'),
  'utf8'
);

const SYSTEM: SystemDescription = {
  hostName: 'TESTBOX',
  osName: 'Test OS',
  osVersion: '1.0',
  kernelVersion: '9.9',
  uptimeSeconds: 60,
  cpu: {
    brand: 'Test CPU X1',
    vendor: 'TestVendor',
    physicalCores: 8,
    logicalCores: 16,
    frequencyMhz: 4500,
  },
  totalMemory: 34_359_738_368,
  totalSwap: 0,
  disks: [],
  interfaces: [],
};

/** What the document is currently wearing. */
function appliedTheme(): string | null {
  return document.documentElement.getAttribute('data-theme');
}

/** The theme in storage, as the boot script would find it. */
function storedTheme(): unknown {
  const stored = window.localStorage.getItem(STORAGE_KEY);
  return stored === null ? null : (JSON.parse(stored) as { theme?: unknown }).theme;
}

/**
 * Everything a relaunch does, in order.
 *
 * A restart is a fresh document that has never been themed, followed by the
 * head script, followed by React. Clearing the attribute first is what makes
 * this a real reload: without it the previous render's attribute would still
 * be there and the boot script could do nothing at all and still "pass".
 */
function relaunch(): { themeAtFirstPaint: string | null } {
  document.documentElement.removeAttribute('data-theme');
  new Function(BOOT_SCRIPT)();
  return { themeAtFirstPaint: appliedTheme() };
}

describe('App › themes', () => {
  beforeEach(() => {
    invoke.mockReset();
    listen.mockReset();
    listen.mockResolvedValue(() => {});
    window.localStorage.clear();
    document.documentElement.removeAttribute('data-theme');
    invoke.mockImplementation((command: string) =>
      Promise.resolve(
        (
          {
            system_description: SYSTEM,
            metrics_history: [],
            process_list: [],
            gpu_devices: [],
          } as Record<string, unknown>
        )[command]
      )
    );
  });

  it.each(THEMES.map((theme) => theme.value))(
    'applies %s to the document and keeps it across a relaunch',
    async (chosen) => {
      const { unmount } = render(<App />);

      await screen.findByText('TESTBOX');
      await userEvent.click(screen.getByTitle('Settings'));

      await userEvent.click(
        await screen.findByRole('radio', {
          name: new RegExp(THEMES.find((theme) => theme.value === chosen)?.label ?? '', 'i'),
        })
      );

      await waitFor(() => {
        expect(appliedTheme()).toBe(chosen);
      });
      expect(storedTheme()).toBe(chosen);

      unmount();

      // The assertion that matters: the theme is on the document from the head
      // script alone, before a single component has rendered. If this were
      // applied by App's effect instead, this value would be null and the real
      // window would open on midnight and then jump.
      expect(relaunch().themeAtFirstPaint).toBe(chosen);

      render(<App />);
      await screen.findByText('TESTBOX');
      expect(appliedTheme()).toBe(chosen);
    }
  );

  it('opens on the default theme when nothing has ever been chosen', async () => {
    expect(relaunch().themeAtFirstPaint).toBeNull();

    render(<App />);
    await screen.findByText('TESTBOX');

    // Null before React, `midnight` after: the unthemed document already wears
    // midnight's values through `@theme`, which is why nothing flashes on a
    // first run either.
    expect(appliedTheme()).toBe('midnight');
  });
});
