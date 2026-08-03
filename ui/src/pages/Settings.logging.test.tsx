/**
 * The logging section of Settings.
 *
 * Kept apart from `Settings.test.tsx`, which covers the preference controls and
 * needs no IPC mock, following `Settings.runtime.test.tsx` and
 * `Settings.models.test.tsx`.
 *
 * Every assertion here is on **content**, not on presence. A test that only
 * checked a radiogroup existed would pass with the three level descriptions
 * rotated by one, which is the realistic way this section breaks: the labels
 * are three words and the descriptions are the entire basis anybody has for
 * choosing between them.
 */

import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { Logging } from '../components/Logging';

const { fetchLogDirectory, logUiEvent, saveLogs, setLogLevel } = vi.hoisted(() => ({
  fetchLogDirectory: vi.fn(),
  logUiEvent: vi.fn(),
  saveLogs: vi.fn(),
  setLogLevel: vi.fn(),
}));

vi.mock('../lib/ipc', () => ({
  fetchLogDirectory,
  logUiEvent,
  saveLogs,
  setLogLevel,
}));

const FOLDER = 'C:\\Users\\sam\\AppData\\Roaming\\osstat\\logs';

beforeEach(() => {
  vi.clearAllMocks();
  fetchLogDirectory.mockResolvedValue(FOLDER);
  setLogLevel.mockResolvedValue(undefined);
  saveLogs.mockResolvedValue(3);
  logUiEvent.mockResolvedValue(undefined);
});

/**
 * Waits for the two effects the section runs on mount.
 *
 * Both resolve a promise and then set state. A synchronous assertion would read
 * the markup before either landed: it would still pass, but React would warn
 * that an update happened outside `act`, and a suite that warns routinely is a
 * suite where the warning that matters goes unread.
 */
async function settled(): Promise<void> {
  await waitFor(() => {
    expect(setLogLevel).toHaveBeenCalled();
  });
  await screen.findByText(FOLDER);
}

describe('Settings › Logs › the level selector', () => {
  it('offers exactly the three levels, in order of how much they capture', async () => {
    render(<Logging level="info" onChangeLevel={vi.fn()} />);
    await settled();

    const names = screen
      .getAllByRole('radio')
      .map((radio) => radio.getAttribute('value'))
      .filter((value) => value !== null);

    expect(names).toEqual(['info', 'debug', 'verbose']);
  });

  it('says what Info captures, in terms of what happened rather than a level name', async () => {
    render(<Logging level="info" onChangeLevel={vi.fn()} />);
    await settled();

    // Named against the description, not merely against the label: swapping two
    // labels moves a description onto the wrong level and this stops matching.
    expect(
      screen.getByRole('radio', { name: /Info.*model sessions, downloads and their outcomes/s })
    ).toBeInTheDocument();
  });

  it('says that Debug adds the page and setting changes Info leaves out', async () => {
    render(<Logging level="info" onChangeLevel={vi.fn()} />);
    await settled();

    expect(
      screen.getByRole('radio', { name: /Debug.*each page you open.*every command that failed/s })
    ).toBeInTheDocument();
  });

  it('warns that Verbose is for reproducing a problem rather than leaving on', async () => {
    // The one thing about Verbose a user has to be told before choosing it: it
    // writes a line every couple of seconds, all day, on an app that lives in
    // the tray.
    render(<Logging level="info" onChangeLevel={vi.fn()} />);
    await settled();

    expect(
      screen.getByRole('radio', {
        name: /Verbose.*every measurement.*reproduce a problem rather than leaving it on/s,
      })
    ).toBeInTheDocument();
  });

  it('shows the stored level as the chosen one', async () => {
    render(<Logging level="debug" onChangeLevel={vi.fn()} />);
    await settled();

    expect(screen.getByRole('radio', { name: /Debug/ })).toBeChecked();
    expect(screen.getByRole('radio', { name: /Info/ })).not.toBeChecked();
  });

  it('reports the chosen level, not merely that something was chosen', async () => {
    const onChangeLevel = vi.fn();
    render(<Logging level="info" onChangeLevel={onChangeLevel} />);
    await settled();

    fireEvent.click(screen.getByRole('radio', { name: /Verbose/ }));

    await waitFor(() => {
      expect(onChangeLevel).toHaveBeenCalledWith('verbose');
    });
  });

  it('re-applies the stored level at startup, because Rust forgets it', async () => {
    render(<Logging level="verbose" onChangeLevel={vi.fn()} />);

    await waitFor(() => {
      expect(setLogLevel).toHaveBeenCalledWith('verbose');
    });
  });

  it('says so when the level could not be applied', async () => {
    // A build with no app-data directory has no log at all. A control that
    // silently appeared to work would be the wrong answer.
    setLogLevel.mockRejectedValue(new Error('log state is not managed'));
    render(<Logging level="debug" onChangeLevel={vi.fn()} />);

    expect(await screen.findByRole('alert')).toHaveTextContent(
      /not applied.*log state is not managed/
    );
  });
});

describe('Settings › Logs › what the section promises', () => {
  it('states that the logs carry no personal data and can be attached to a bug report', async () => {
    // The user-visible payoff of the whole design, said where somebody is
    // deciding whether to send their log to a stranger on an issue tracker.
    render(<Logging level="info" onChangeLevel={vi.fn()} />);

    const section = await screen.findByRole('region', { name: 'Logs' });

    expect(section).toHaveTextContent(/no personal data/);
    expect(section).toHaveTextContent(/safe to attach to a bug report/);
  });

  it('names what is absent, so the promise can be checked rather than trusted', async () => {
    render(<Logging level="info" onChangeLevel={vi.fn()} />);
    await settled();

    const section = screen.getByRole('region', { name: 'Logs' });

    for (const absent of ['file names', 'paths', 'process names', 'addresses']) {
      expect(section).toHaveTextContent(absent);
    }
  });

  it('states the retention, so a week-old problem is known to be gone', async () => {
    render(<Logging level="info" onChangeLevel={vi.fn()} />);
    await settled();

    expect(screen.getByRole('region', { name: 'Logs' })).toHaveTextContent(
      /week of daily log files.*oldest/s
    );
  });
});

describe('Settings › Logs › saving', () => {
  it('shows where the logs are, so they can be found without the app', async () => {
    render(<Logging level="info" onChangeLevel={vi.fn()} />);

    expect(await screen.findByText(FOLDER)).toBeInTheDocument();
  });

  it('copies into the folder that was typed', async () => {
    render(<Logging level="info" onChangeLevel={vi.fn()} />);
    await screen.findByText(FOLDER);

    fireEvent.change(screen.getByLabelText('Copy the logs to'), {
      target: { value: 'D:\\reports' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save logs' }));

    await waitFor(() => {
      expect(saveLogs).toHaveBeenCalledWith('D:\\reports');
    });
  });

  it('says how many files were copied and where, not just that it worked', async () => {
    saveLogs.mockResolvedValue(3);
    render(<Logging level="info" onChangeLevel={vi.fn()} />);
    await screen.findByText(FOLDER);

    fireEvent.change(screen.getByLabelText('Copy the logs to'), {
      target: { value: 'D:\\reports' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save logs' }));

    expect(await screen.findByRole('status')).toHaveTextContent('Copied 3 files to D:\\reports.');
  });

  it('counts one file as a file rather than as 1 files', async () => {
    saveLogs.mockResolvedValue(1);
    render(<Logging level="info" onChangeLevel={vi.fn()} />);
    await screen.findByText(FOLDER);

    fireEvent.change(screen.getByLabelText('Copy the logs to'), {
      target: { value: 'D:\\reports' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save logs' }));

    expect(await screen.findByRole('status')).toHaveTextContent('Copied 1 file to');
  });

  it('will not save into nothing', async () => {
    render(<Logging level="info" onChangeLevel={vi.fn()} />);
    await screen.findByText(FOLDER);

    expect(screen.getByRole('button', { name: 'Save logs' })).toBeDisabled();
  });

  it('reports a failed save and claims nothing was copied', async () => {
    saveLogs.mockRejectedValue(new Error('the logs could not be copied: access is denied'));
    render(<Logging level="info" onChangeLevel={vi.fn()} />);
    await screen.findByText(FOLDER);

    fireEvent.change(screen.getByLabelText('Copy the logs to'), {
      target: { value: 'D:\\reports' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save logs' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('access is denied');
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });

  it('forwards a failed command into the one log rather than only the screen', async () => {
    saveLogs.mockRejectedValue(new Error('access is denied'));
    render(<Logging level="info" onChangeLevel={vi.fn()} />);
    await screen.findByText(FOLDER);

    fireEvent.change(screen.getByLabelText('Copy the logs to'), {
      target: { value: 'D:\\reports' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save logs' }));

    await waitFor(() => {
      expect(logUiEvent).toHaveBeenCalledWith('commandFailed');
    });
  });

  it('offers nothing to save into when the log folder could not be read', async () => {
    fetchLogDirectory.mockRejectedValue(new Error('no app data directory'));
    render(<Logging level="info" onChangeLevel={vi.fn()} />);

    expect(await screen.findByRole('alert')).toHaveTextContent(
      /Could not read the log folder.*no app data directory/
    );
    expect(screen.queryByRole('button', { name: 'Save logs' })).not.toBeInTheDocument();
  });
});
