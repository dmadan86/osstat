/**
 * The confirmation flow's behaviour, mocked at the IPC boundary.
 *
 * Exists to pin the decisions that are easy to undo by accident: nothing is
 * signalled before a confirmation is accepted, a critical process gets a
 * second, differently worded confirmation, the forceful step waits for the
 * grace period unless Windows already says there is nothing to wait for, and
 * the two backend refusals read as what they are rather than as one generic
 * failure.
 */

import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { EndProcessDialog, type EndProcessDialogProps } from './EndProcessDialog';
import type { CriticalProcess } from '../bindings/CriticalProcess';
import type { ProcessDiff } from '../bindings/ProcessDiff';
import type { Termination } from '../bindings/Termination';
import { GRACE_MS } from '../lib/termination';

const { fetchCriticalProcesses, onProcessesTick, terminateProcess } = vi.hoisted(() => ({
  fetchCriticalProcesses: vi.fn(),
  onProcessesTick: vi.fn(),
  terminateProcess: vi.fn(),
}));

vi.mock('../lib/ipc', () => ({
  fetchCriticalProcesses,
  onProcessesTick,
  terminateProcess,
}));

/** Captures the handler a subscription was given, so a test can fire it. */
function capturing<T>(): {
  fire: (payload: T) => void;
  subscribe: (h: (p: T) => void) => Promise<() => void>;
} {
  let handler: ((payload: T) => void) | null = null;

  return {
    fire: (payload) => handler?.(payload),
    subscribe: (h) => {
      handler = h;
      return Promise.resolve(() => {
        handler = null;
      });
    },
  };
}

const TARGET: EndProcessDialogProps['target'] = {
  key: { pid: 4242, startedAt: 1000 },
  name: 'chrome',
  exe: 'C:\\Program Files\\Chrome\\chrome.exe',
};

const EXPLORER: EndProcessDialogProps['target'] = {
  key: { pid: 99, startedAt: 500 },
  name: 'explorer.exe',
  exe: 'C:\\Windows\\explorer.exe',
};

const CRITICAL_LIST: CriticalProcess[] = [
  {
    platform: 'windows',
    name: 'explorer.exe',
    paths: ['\\Windows\\explorer.exe'],
    breaks: 'the taskbar, the Start menu and the desktop',
  },
];

function renderDialog(overrides: Partial<EndProcessDialogProps> = {}): {
  onClose: ReturnType<typeof vi.fn>;
} {
  const onClose = vi.fn();
  render(<EndProcessDialog target={TARGET} onClose={onClose} {...overrides} />);
  return { onClose };
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.useRealTimers();
  fetchCriticalProcesses.mockResolvedValue([]);
  onProcessesTick.mockResolvedValue(() => undefined);
  terminateProcess.mockResolvedValue('signalled' satisfies Termination);
});

describe('EndProcessDialog', () => {
  it('ends nothing before the confirmation is accepted', async () => {
    renderDialog();

    await screen.findByRole('dialog', { name: /end chrome/i });

    expect(terminateProcess).not.toHaveBeenCalled();
  });

  it('asks a second, differently worded time for a critical process', async () => {
    fetchCriticalProcesses.mockResolvedValue(CRITICAL_LIST);
    renderDialog({ target: EXPLORER });

    const firstPrompt = (await screen.findByRole('dialog')).textContent;
    fireEvent.click(screen.getByRole('button', { name: /^end explorer\.exe$/i }));

    const secondPrompt = await screen.findByText(/are you sure you want to end it anyway/i);
    expect(secondPrompt.closest('[role="dialog"]')?.textContent).not.toBe(firstPrompt);
    expect(terminateProcess).not.toHaveBeenCalled();
  });

  it('names what breaks when the target is critical', async () => {
    fetchCriticalProcesses.mockResolvedValue(CRITICAL_LIST);
    renderDialog({ target: EXPLORER });

    fireEvent.click(await screen.findByRole('button', { name: /^end explorer\.exe$/i }));

    expect(
      await screen.findByText(/the taskbar, the start menu and the desktop/i)
    ).toBeInTheDocument();
  });

  it('warns that children keep running', async () => {
    renderDialog();

    await screen.findByRole('dialog');

    expect(screen.getByText(/keep running on their own/i)).toBeInTheDocument();
  });

  it('offers the forceful step only after the graceful one has had its grace period', async () => {
    // `shouldAdvanceTime` lets React's own effect scheduling keep moving in
    // real time — without it, the component's promise chain (fetching the
    // critical list, sending the graceful request) never settles, because
    // that scheduling itself leans on timers this would otherwise freeze.
    // The grace timer is then fast-forwarded on top explicitly.
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { onClose } = renderDialog();

    fireEvent.click(await screen.findByRole('button', { name: /^end chrome$/i }));
    await screen.findByRole('status');

    expect(screen.queryByRole('button', { name: /force end/i })).not.toBeInTheDocument();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(GRACE_MS);
    });

    expect(screen.getByRole('button', { name: /force end chrome/i })).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();
    vi.useRealTimers();
  });

  it('skips the wait entirely when Windows reports no window to close', async () => {
    terminateProcess.mockResolvedValue('noWindowToClose' satisfies Termination);
    renderDialog();

    fireEvent.click(await screen.findByRole('button', { name: /^end chrome$/i }));

    expect(await screen.findByRole('button', { name: /force end chrome/i })).toBeInTheDocument();
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });

  it('closes without a second prompt when the process disappears from the tick', async () => {
    const tick = capturing<ProcessDiff>();
    onProcessesTick.mockImplementation(tick.subscribe);
    const { onClose } = renderDialog();

    fireEvent.click(await screen.findByRole('button', { name: /^end chrome$/i }));
    await screen.findByRole('status');

    await act(async () => {
      tick.fire({
        added: [],
        removed: [{ pid: TARGET.key.pid, startedAt: TARGET.key.startedAt }],
        changed: [],
      });
    });

    expect(onClose).toHaveBeenCalled();
  });

  it('explains an identity mismatch as the process already being gone', async () => {
    terminateProcess.mockRejectedValue({
      message: 'pid 4242 has been reused by a different process; nothing was signalled',
      permissionDenied: false,
      identityMismatch: true,
    });
    renderDialog();

    fireEvent.click(await screen.findByRole('button', { name: /^end chrome$/i }));

    expect(await screen.findByText(/chrome has already ended/i)).toBeInTheDocument();
  });

  it('explains a permission failure without offering elevation', async () => {
    terminateProcess.mockRejectedValue({
      message: 'cannot signal pid 4242',
      permissionDenied: true,
      identityMismatch: false,
    });
    renderDialog();

    fireEvent.click(await screen.findByRole('button', { name: /^end chrome$/i }));

    expect(await screen.findByText(/belongs to another user/i)).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: /elevate|administrator|retry/i })
    ).not.toBeInTheDocument();
  });
});
