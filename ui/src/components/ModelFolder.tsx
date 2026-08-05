/**
 * The model-folder section of Settings.
 *
 * Downloaded models are the largest thing osstat ever puts on a disk — a single
 * 70B file is tens of gigabytes — so where they land has to be the user's
 * choice rather than a hard-coded corner of the app-data directory.
 *
 * Two decisions are visible in the markup and worth stating:
 *
 * **Changing the folder never moves anything by itself.** The move is asked
 * for, with the count and the total size from `models_plan_move` in the
 * question, because silently relocating 40 GB in response to a corrected typo
 * is the worst thing this row could do. Declining the move still changes the
 * setting: records carry absolute paths, so the files already downloaded stay
 * runnable wherever they are.
 *
 * **The folder is typed, not chosen from a native dialog.** A folder picker
 * would mean a new Tauri plugin on both sides of the boundary, and the webview
 * gaining a second way to name a path. `Chat.tsx` takes a model path the same
 * way for the same reason.
 *
 * Its own file rather than another section inside `Settings.tsx`, matching
 * {@link InferenceRuntime}: this one owns three event subscriptions and a
 * confirmation, where every other section is a control bound to a preference.
 */

import { useEffect, useState } from 'react';

import type { LibraryMovePlan } from '../bindings/LibraryMovePlan';
import type { ModelProgress } from '../bindings/ModelProgress';
import { formatBytes } from '../lib/format';
import { ROW_CLASS, SettingHeader } from './SettingRow';
import {
  fetchModelFolder,
  moveModelLibrary,
  onModelDone,
  onModelFailed,
  onModelProgress,
  planModelMove,
  setModelFolder,
} from '../lib/ipc';

/** What the section knows about where models live. */
type LoadState =
  | { status: 'loading' }
  | { status: 'ready'; folder: string }
  | { status: 'error'; message: string };

/** A folder the user has asked for, and what moving into it would cost. */
interface Proposal {
  /** Where they want models kept from now on. */
  path: string;
  /** How many files and bytes are currently elsewhere. */
  plan: LibraryMovePlan;
}

/** Renders an unknown thrown value as a message. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** How many files, in words that read correctly at one. */
function countedFiles(files: number): string {
  return files === 1 ? '1 file' : `${String(files)} files`;
}

/** Renders the model-folder section. */
export function ModelFolder(): React.JSX.Element {
  const [state, setState] = useState<LoadState>({ status: 'loading' });
  const [draft, setDraft] = useState('');
  const [proposal, setProposal] = useState<Proposal | null>(null);
  const [moving, setMoving] = useState<ModelProgress | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [reloadToken, setReloadToken] = useState(0);

  useEffect(() => {
    let cancelled = false;

    fetchModelFolder().then(
      (folder) => {
        if (cancelled) return;

        // An answer that is not a path is treated as a failure to read rather
        // than as an empty one: `""` in the picker would offer to move the
        // whole library into the working directory. `InferenceRuntime` checks
        // its own answer the same way and for the same reason.
        if (typeof folder === 'string' && folder !== '') {
          setState({ status: 'ready', folder });
          setDraft(folder);
        } else {
          setState({ status: 'error', message: 'the backend named no folder' });
        }
      },
      (error: unknown) => {
        if (!cancelled) setState({ status: 'error', message: messageOf(error) });
      }
    );

    return () => {
      cancelled = true;
    };
  }, [reloadToken]);

  useEffect(() => {
    const subscriptions = [
      onModelProgress((next) => {
        // A `null` key is the whole library moving; a cell's key is a download,
        // which belongs to the advisor rather than to this row.
        if (next.key === null) setMoving(next);
      }),
      onModelDone((next) => {
        if (next.key !== null) return;
        setMoving(null);
        setReloadToken((token) => token + 1);
      }),
      onModelFailed((next) => {
        if (next.key !== null) return;
        setMoving(null);
        setProblem(next.message);
        setReloadToken((token) => token + 1);
      }),
    ];

    return () => {
      for (const subscription of subscriptions) {
        subscription.then(
          (off) => {
            off();
          },
          () => {
            // Nothing to unsubscribe from if the subscription itself failed.
          }
        );
      }
    };
  }, []);

  /** Points new downloads at `path` without touching what is already on disk. */
  function keepFolder(path: string): void {
    setProposal(null);
    setModelFolder(path).then(
      () => {
        setState({ status: 'ready', folder: path });
        setProblem(null);
      },
      (error: unknown) => {
        setProblem(messageOf(error));
      }
    );
  }

  /** Asks what moving into `draft` would cost, then asks the user. */
  function propose(): void {
    const path = draft.trim();
    if (path === '' || moving !== null) return;

    setProblem(null);
    planModelMove(path).then(
      (plan) => {
        // Nothing to move means nothing to ask about, and a confirmation over
        // zero files trains people to click through the one that matters.
        if (plan.files === 0) keepFolder(path);
        else setProposal({ path, plan });
      },
      (error: unknown) => {
        setProblem(messageOf(error));
      }
    );
  }

  /** Moves every downloaded file into `path`, which also points new ones there. */
  function move(path: string): void {
    setProposal(null);
    setProblem(null);
    setMoving({
      key: null,
      phase: 'moving',
      downloadedBytes: 0,
      totalBytes: 0,
      // A move reports neither. A same-volume move renames rather than copies,
      // so a byte rate would describe an operation that never touched the
      // bytes — the backend says the same thing by sending `null` here.
      bytesPerSecond: null,
      secondsRemaining: null,
    });

    moveModelLibrary(path).then(
      () => {
        setState({ status: 'ready', folder: path });
      },
      (error: unknown) => {
        setMoving(null);
        setProblem(messageOf(error));
      }
    );
  }

  const current = state.status === 'ready' ? state.folder : null;

  return (
    <section aria-label="Where models are kept" className={ROW_CLASS}>
      <SettingHeader icon="folder" label="Model folder">
        Where downloaded models are kept. A single model is gigabytes, so this can be pointed at
        whichever disk has room. Nothing is created until the first download.
      </SettingHeader>

      {state.status === 'loading' && (
        <p className="mt-3 text-xs text-text-muted">Reading where models are kept…</p>
      )}

      {state.status === 'error' && (
        <p role="alert" className="mt-3 text-xs text-red-400">
          Could not read the model folder: {state.message}
        </p>
      )}

      {current !== null && (
        <>
          <p data-selectable className="mt-3 break-all font-mono text-xs text-text">
            {current}
          </p>

          <div className="mt-2 flex flex-wrap items-end gap-2">
            <div className="flex min-w-0 flex-1 flex-col gap-1">
              <label
                htmlFor="model-folder"
                className="text-[10px] uppercase tracking-wider text-text-muted"
              >
                Model folder
              </label>
              <input
                id="model-folder"
                type="text"
                value={draft}
                spellCheck={false}
                onChange={(event) => {
                  setDraft(event.target.value);
                }}
                className="w-full rounded-md border border-edge bg-transparent px-2 py-1 font-mono text-xs"
              />
            </div>
            <button
              type="button"
              disabled={draft.trim() === '' || draft.trim() === current || moving !== null}
              onClick={propose}
              className="rounded-md border border-edge px-2.5 py-1 text-xs text-text hover:bg-white/[0.04] disabled:opacity-40"
            >
              Use this folder
            </button>
          </div>
        </>
      )}

      {proposal !== null && (
        <MoveQuestion
          proposal={proposal}
          onMove={() => {
            move(proposal.path);
          }}
          onKeep={() => {
            keepFolder(proposal.path);
          }}
        />
      )}

      {moving !== null && (
        <p role="status" className="mt-3 font-mono text-xs text-text-muted">
          Moving the library
          {moving.totalBytes > 0 &&
            ` — ${formatBytes(moving.downloadedBytes)} of ${formatBytes(moving.totalBytes)}`}
          …
        </p>
      )}

      {problem !== null && (
        <p role="alert" className="mt-3 text-xs text-amber-400/90">
          {problem}
        </p>
      )}
    </section>
  );
}

/** The question asked before any bytes move. */
function MoveQuestion({
  proposal,
  onMove,
  onKeep,
}: {
  proposal: Proposal;
  onMove: () => void;
  onKeep: () => void;
}): React.JSX.Element {
  const { path, plan } = proposal;

  return (
    <div
      role="alertdialog"
      aria-label="Move the model library"
      className="mt-3 rounded-lg border border-amber-500/40 bg-amber-500/5 p-3"
    >
      <p className="text-xs text-text">
        {countedFiles(plan.files)}, {formatBytes(plan.bytes)}, are in the old folder. Move them to{' '}
        <span data-selectable className="font-mono break-all">
          {path}
        </span>
        ?
      </p>

      <p className="mt-1 text-[11px] text-text-muted">
        {plan.sameVolume
          ? 'Everything is on the same volume, so each file is renamed rather than copied — this takes moments.'
          : 'This is a different volume, so each file is copied and verified before the original is removed. Nothing is deleted until its copy has verified, and a library this size can take a long time.'}
      </p>

      <div className="mt-2 flex flex-wrap gap-2">
        <button
          type="button"
          onClick={onMove}
          className="rounded-md border border-accent px-2.5 py-1 text-xs text-accent hover:bg-accent/10"
        >
          Move them
        </button>
        <button
          type="button"
          onClick={onKeep}
          className="rounded-md border border-edge px-2.5 py-1 text-xs text-text hover:bg-white/[0.04]"
        >
          Leave them where they are
        </button>
      </div>

      <p className="mt-2 text-[11px] text-text-faint">
        Leaving them still points new downloads at the new folder. Every record carries an absolute
        path, so models already downloaded stay runnable wherever they sit.
      </p>
    </div>
  );
}
