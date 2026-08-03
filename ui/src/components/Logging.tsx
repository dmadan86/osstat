/**
 * The logging section of Settings.
 *
 * This is where the whole design pays off for the user, so the section says the
 * thing out loud: **the log contains no personal data and is safe to attach to
 * a bug report.** That sentence belongs here rather than only in SECURITY.md,
 * because here is where somebody is deciding whether to send their log to a
 * stranger on an issue tracker, and a promise they have to go and find is one
 * they will reasonably assume does not exist.
 *
 * Two decisions are visible in the markup:
 *
 * **Each level carries its own description.** Every other setting on this page
 * is a row of labelled buttons under one shared explanation, which works when
 * the labels speak for themselves. "Info", "Debug" and "Verbose" do not — they
 * differ in what they *capture*, and that is the only basis anybody has for
 * choosing between them.
 *
 * **The destination folder is typed, not chosen from a native dialog.** The
 * same decision {@link ModelFolder} documents, for the same reason: a folder
 * picker would mean a new Tauri plugin on both sides of the boundary and a
 * second way for the webview to name a path.
 *
 * Its own file rather than another section inside `Settings.tsx`, matching
 * {@link InferenceRuntime} and {@link ModelFolder}: this one owns an effect, a
 * command that can fail, and state of its own.
 */

import { useEffect, useState } from 'react';

import type { LogLevel } from '../bindings/LogLevel';
import { fetchLogDirectory, logUiEvent, saveLogs, setLogLevel } from '../lib/ipc';
import { CHOICES } from '../lib/preferences';

/** What the section knows about where the logs are. */
type FolderState =
  | { status: 'loading' }
  | { status: 'ready'; folder: string }
  | { status: 'error'; message: string };

/** What the last attempt to save the logs produced. */
type SaveState =
  | { status: 'idle' }
  | { status: 'saving' }
  | { status: 'saved'; files: number; folder: string }
  | { status: 'error'; message: string };

/** Renders an unknown thrown value as a message. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** How many files, in words that read correctly at one. */
function countedFiles(files: number): string {
  return files === 1 ? '1 file' : `${String(files)} files`;
}

/** What the logging section needs. */
export interface LoggingProps {
  /** How much detail the log is set to carry. */
  level: LogLevel;
  /** Applies a new level. */
  onChangeLevel: (level: LogLevel) => void;
}

/** Renders the logging section. */
export function Logging({ level, onChangeLevel }: LoggingProps): React.JSX.Element {
  const [folder, setFolder] = useState<FolderState>({ status: 'loading' });
  const [destination, setDestination] = useState('');
  const [save, setSave] = useState<SaveState>({ status: 'idle' });
  const [unapplied, setUnapplied] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    fetchLogDirectory().then(
      (path) => {
        if (cancelled) return;

        // An answer that is not a path is a failure to read rather than an
        // empty folder, exactly as `ModelFolder` treats its own: `""` here
        // would offer to copy the logs into the working directory.
        if (typeof path === 'string' && path !== '') setFolder({ status: 'ready', folder: path });
        else setFolder({ status: 'error', message: 'the backend named no folder' });
      },
      (error: unknown) => {
        if (!cancelled) setFolder({ status: 'error', message: messageOf(error) });
      }
    );

    return () => {
      cancelled = true;
    };
  }, []);

  // Replayed on every render where the stored level changes, which includes the
  // first: Rust holds only what it was last told, so this is what makes the
  // setting survive a restart. Same arrangement as the close behaviour.
  useEffect(() => {
    let cancelled = false;

    setLogLevel(level).then(
      () => {
        if (!cancelled) setUnapplied(null);
      },
      (error: unknown) => {
        // Worth showing rather than swallowing. A build whose app-data
        // directory could not be resolved has no log at all, and a level
        // control that silently appeared to work would be the wrong answer.
        if (!cancelled) setUnapplied(messageOf(error));
      }
    );

    return () => {
      cancelled = true;
    };
  }, [level]);

  /** Copies the logs into the typed folder. */
  function copyInto(path: string): void {
    setSave({ status: 'saving' });

    saveLogs(path).then(
      (files) => {
        setSave({ status: 'saved', files, folder: path });
      },
      (error: unknown) => {
        setSave({ status: 'error', message: messageOf(error) });
        logUiEvent('commandFailed').catch(() => {
          // Reporting that a command failed must not itself be able to fail
          // loudly; there is nowhere left to report it to.
        });
      }
    );
  }

  const current = folder.status === 'ready' ? folder.folder : null;

  return (
    <section aria-label="Logs" className="border-t border-edge p-3">
      <p className="text-sm">Logs</p>
      <p className="mt-0.5 text-xs text-neutral-500">
        osstat keeps a week of daily log files, deleting the oldest as new ones arrive.{' '}
        <strong className="font-medium text-neutral-300">
          They contain no personal data — no file names, paths, process names, addresses, or
          anything you typed — so they are safe to attach to a bug report.
        </strong>
      </p>

      <div role="radiogroup" aria-label="Log detail" className="mt-3 flex flex-col gap-1.5">
        {CHOICES.logLevel.map((choice) => (
          <label
            key={choice.value}
            className="flex cursor-pointer items-start gap-2 rounded-md border border-edge px-2.5 py-1.5 hover:bg-white/[0.04]"
          >
            <input
              type="radio"
              name="log-level"
              value={choice.value}
              checked={choice.value === level}
              onChange={() => {
                onChangeLevel(choice.value);
              }}
              className="mt-0.5 accent-accent"
            />
            <span className="min-w-0">
              <span className="block text-xs text-neutral-200">{choice.label}</span>
              <span className="block text-[11px] text-neutral-500">{choice.description}</span>
            </span>
          </label>
        ))}
      </div>

      {unapplied !== null && (
        <p role="alert" className="mt-2 text-xs text-amber-400/90">
          That level was not applied: {unapplied}
        </p>
      )}

      {folder.status === 'loading' && (
        <p className="mt-3 text-xs text-neutral-500">Reading where the logs are kept…</p>
      )}

      {folder.status === 'error' && (
        <p role="alert" className="mt-3 text-xs text-red-400">
          Could not read the log folder: {folder.message}
        </p>
      )}

      {current !== null && (
        <>
          <p data-selectable className="mt-3 break-all font-mono text-xs text-neutral-300">
            {current}
          </p>

          <div className="mt-2 flex flex-wrap items-end gap-2">
            <div className="flex min-w-0 flex-1 flex-col gap-1">
              <label
                htmlFor="log-destination"
                className="text-[10px] uppercase tracking-wider text-neutral-500"
              >
                Copy the logs to
              </label>
              <input
                id="log-destination"
                type="text"
                value={destination}
                spellCheck={false}
                onChange={(event) => {
                  setDestination(event.target.value);
                }}
                className="w-full rounded-md border border-edge bg-transparent px-2 py-1 font-mono text-xs"
              />
            </div>
            <button
              type="button"
              disabled={destination.trim() === '' || save.status === 'saving'}
              onClick={() => {
                copyInto(destination.trim());
              }}
              className="rounded-md border border-edge px-2.5 py-1 text-xs text-neutral-300 hover:bg-white/[0.04] disabled:opacity-40"
            >
              Save logs
            </button>
          </div>
        </>
      )}

      {save.status === 'saved' && (
        <p role="status" className="mt-2 text-xs text-neutral-400">
          Copied {countedFiles(save.files)} to{' '}
          <span data-selectable className="font-mono break-all">
            {save.folder}
          </span>
          .
        </p>
      )}

      {save.status === 'error' && (
        <p role="alert" className="mt-2 text-xs text-amber-400/90">
          {save.message}
        </p>
      )}
    </section>
  );
}
