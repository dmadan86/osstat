/**
 * The model-folder section of Settings.
 *
 * Kept apart from `Settings.test.tsx` for the same reason
 * `Settings.runtime.test.tsx` is: those cover preference controls and need no
 * IPC mock at all.
 *
 * The decision these tests pin is that changing the folder never moves
 * gigabytes on its own. A picker that silently relocated a 40 GB library
 * because someone corrected a typo would be the worst thing this row could do,
 * so the count and the size are stated and the move is asked for.
 */

import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { ModelFolder } from '../components/ModelFolder';
import type { LibraryMovePlan } from '../bindings/LibraryMovePlan';
import type { ModelDone } from '../bindings/ModelDone';
import type { ModelFailure } from '../bindings/ModelFailure';
import type { ModelProgress } from '../bindings/ModelProgress';

const {
  fetchModelFolder,
  moveModelLibrary,
  onModelDone,
  onModelFailed,
  onModelProgress,
  planModelMove,
  setModelFolder,
} = vi.hoisted(() => ({
  fetchModelFolder: vi.fn(),
  moveModelLibrary: vi.fn(),
  onModelDone: vi.fn(),
  onModelFailed: vi.fn(),
  onModelProgress: vi.fn(),
  planModelMove: vi.fn(),
  setModelFolder: vi.fn(),
}));

vi.mock('../lib/ipc', () => ({
  fetchModelFolder,
  moveModelLibrary,
  onModelDone,
  onModelFailed,
  onModelProgress,
  planModelMove,
  setModelFolder,
}));

/** Captures the handler a subscription was given, so a test can fire it. */
function capturing<T>(): {
  fire: (payload: T) => void;
  subscribe: (handler: (payload: T) => void) => Promise<() => void>;
} {
  let held: ((payload: T) => void) | null = null;

  return {
    fire: (payload) => held?.(payload),
    subscribe: (handler) => {
      held = handler;
      return Promise.resolve(() => {
        held = null;
      });
    },
  };
}

const progress = capturing<ModelProgress>();
const done = capturing<ModelDone>();
const failed = capturing<ModelFailure>();

const CURRENT = 'C:\\Users\\me\\AppData\\Roaming\\osstat\\models';
const CHOSEN = 'D:\\models';

function plan(overrides: Partial<LibraryMovePlan> = {}): LibraryMovePlan {
  return { files: 2, bytes: 3072, sameVolume: false, ...overrides };
}

/** Types a new folder into the picker and asks for it. */
async function choose(user: ReturnType<typeof userEvent.setup>, path: string): Promise<void> {
  await user.clear(screen.getByLabelText(/model folder/i));
  await user.type(screen.getByLabelText(/model folder/i), path);
  await user.click(screen.getByRole('button', { name: /use this folder/i }));
}

beforeEach(() => {
  vi.clearAllMocks();
  fetchModelFolder.mockResolvedValue(CURRENT);
  planModelMove.mockResolvedValue(plan());
  setModelFolder.mockResolvedValue(undefined);
  moveModelLibrary.mockResolvedValue(undefined);
  onModelProgress.mockImplementation(progress.subscribe);
  onModelDone.mockImplementation(done.subscribe);
  onModelFailed.mockImplementation(failed.subscribe);
});

describe('Settings › model folder', () => {
  it('shows the folder downloads currently land in', async () => {
    render(<ModelFolder />);

    expect(await screen.findByText(CURRENT)).toBeInTheDocument();
  });

  it('states the count and size before moving the library', async () => {
    // "Change the model folder?" is not a question anybody can answer. "Move 2
    // files, 3.00 KB, onto a different volume" is.
    const user = userEvent.setup();
    render(<ModelFolder />);
    await screen.findByText(CURRENT);

    await choose(user, CHOSEN);

    const question = await screen.findByRole('alertdialog', { name: /move the model library/i });
    expect(question).toHaveTextContent(/2 files/);
    expect(question).toHaveTextContent(/3\.00 KB/);
    expect(question).toHaveTextContent(new RegExp(CHOSEN.replace(/\\/g, '\\\\')));

    // Asked, not done.
    expect(moveModelLibrary).not.toHaveBeenCalled();
    expect(setModelFolder).not.toHaveBeenCalled();
  });

  it('says a cross-volume move copies and verifies rather than renames', async () => {
    // Instant against an hour. The user is agreeing to a wait, so the wait is
    // named before they agree to it.
    const user = userEvent.setup();
    render(<ModelFolder />);
    await screen.findByText(CURRENT);

    await choose(user, CHOSEN);

    expect(await screen.findByRole('alertdialog')).toHaveTextContent(/copied and verified/i);
  });

  it('moves the library only when the move is confirmed', async () => {
    const user = userEvent.setup();
    render(<ModelFolder />);
    await screen.findByText(CURRENT);
    await choose(user, CHOSEN);

    const question = await screen.findByRole('alertdialog');
    await user.click(within(question).getByRole('button', { name: /move them/i }));

    await waitFor(() => {
      expect(moveModelLibrary).toHaveBeenCalledWith(CHOSEN);
    });
  });

  it('leaves the files where they are and still points new downloads at the new folder', async () => {
    // Declining the move is not declining the setting. The records carry
    // absolute paths, so the old files stay runnable either way.
    const user = userEvent.setup();
    render(<ModelFolder />);
    await screen.findByText(CURRENT);
    await choose(user, CHOSEN);

    const question = await screen.findByRole('alertdialog');
    await user.click(within(question).getByRole('button', { name: /leave them/i }));

    await waitFor(() => {
      expect(setModelFolder).toHaveBeenCalledWith(CHOSEN);
    });
    expect(moveModelLibrary).not.toHaveBeenCalled();
  });

  it('does not ask about moving an empty library', async () => {
    // Nothing to move means nothing to ask about, and a confirmation over zero
    // files trains people to click through the one that matters.
    const user = userEvent.setup();
    planModelMove.mockResolvedValue(plan({ files: 0, bytes: 0, sameVolume: true }));
    render(<ModelFolder />);
    await screen.findByText(CURRENT);

    await choose(user, CHOSEN);

    await waitFor(() => {
      expect(setModelFolder).toHaveBeenCalledWith(CHOSEN);
    });
    expect(screen.queryByRole('alertdialog')).toBeNull();
  });
});
