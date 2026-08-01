/**
 * The confirmation flow for ending a process, shared by every page that
 * offers it.
 *
 * Renders through a small sequence of states rather than one flat form:
 * `confirm` asks once, `criticalConfirm` asks again — differently worded —
 * only when the target matches the critical-process list, `waiting` covers
 * the gap after a graceful request while `processes:tick` might report the
 * process gone on its own, and `force` is offered once the grace period
 * passes without that happening. `error` covers what the backend refused.
 *
 * Subscribes to `onProcessesTick` directly rather than taking the stream from
 * whichever page opened it: `Ports.tsx` fetches on demand and has no stream
 * of its own, but the `processes:tick` event reaches the window regardless of
 * which page is visible, so this is the one place termination needs to watch
 * it — for the dialog's whole lifetime, not only the `waiting` phase, so a
 * process that exits on its own while the user is still reading the
 * confirmation closes the dialog too rather than asking about something
 * already gone.
 *
 * Written after `InferenceRuntime.tsx`'s shape: module-scope async loaders
 * that return the next state rather than setting it, `setState` only inside
 * a promise callback, and subscriptions cleaned up in the effect's return.
 */

import { useEffect, useRef, useState } from 'react';

import type { CriticalProcess } from '../bindings/CriticalProcess';
import { fetchCriticalProcesses, onProcessesTick, terminateProcess } from '../lib/ipc';
import {
  GRACE_MS,
  matchCritical,
  WAIT_CEILING_MS,
  type EndProcessTarget,
} from '../lib/termination';

export type { EndProcessTarget } from '../lib/termination';

/** What the dialog is doing right now. */
type DialogState =
  | { phase: 'confirm' }
  | { phase: 'criticalConfirm'; critical: CriticalProcess }
  | { phase: 'waiting' }
  | { phase: 'force' }
  | { phase: 'error'; message: string; identityMismatch: boolean; permissionDenied: boolean };

/** The shape `terminate_process` rejects with, read structurally. */
interface TerminationErrorLike {
  message: unknown;
  identityMismatch: unknown;
  permissionDenied: unknown;
}

function isTerminationError(error: unknown): error is TerminationErrorLike {
  return (
    typeof error === 'object' &&
    error !== null &&
    'message' in error &&
    'identityMismatch' in error &&
    'permissionDenied' in error
  );
}

/** Turns whatever `terminateProcess` rejected with into the error state's fields. */
function describeError(error: unknown): Omit<Extract<DialogState, { phase: 'error' }>, 'phase'> {
  if (isTerminationError(error)) {
    return {
      message: typeof error.message === 'string' ? error.message : String(error.message),
      identityMismatch: error.identityMismatch === true,
      permissionDenied: error.permissionDenied === true,
    };
  }
  return {
    message: error instanceof Error ? error.message : String(error),
    identityMismatch: false,
    permissionDenied: false,
  };
}

/**
 * Fetches the critical list and matches it against a target.
 *
 * Module scope, returning the next phase rather than setting it — see
 * `loadRuntimeStatus` in `InferenceRuntime.tsx` for the same shape.
 */
async function loadCriticalMatch(target: EndProcessTarget): Promise<DialogState> {
  const list = await fetchCriticalProcesses();
  const critical = matchCritical(list, target.name, target.exe);
  return critical === null ? { phase: 'waiting' } : { phase: 'criticalConfirm', critical };
}

/** Props for {@link EndProcessDialog}. */
export interface EndProcessDialogProps {
  /** The process being asked about. */
  target: EndProcessTarget;
  /** Called once the dialog has nothing further to do: cancelled, failed, or the process is confirmed gone. */
  onClose: () => void;
}

/** The confirmation dialog for ending a process. */
export function EndProcessDialog({ target, onClose }: EndProcessDialogProps): React.JSX.Element {
  const [state, setState] = useState<DialogState>({ phase: 'confirm' });
  const dialogRef = useRef<HTMLDivElement>(null);

  // Moves focus to the dialog's least destructive control — Cancel, the
  // first button in every phase that has one — as soon as it opens, so the
  // keyboard starts inside the dialog instead of wherever it was on the page
  // behind it. Mount-only is enough: App.tsx keys this component on the
  // target's identity, so a new target is a new mount, never a prop change
  // on this one.
  useEffect(() => {
    dialogRef.current?.querySelector<HTMLButtonElement>('button')?.focus();
  }, []);

  // Escape closes the dialog without terminating anything, from any phase.
  // The overlay alone blocks the mouse but not the keyboard, and closing is
  // always safe — no phase's Cancel/Close button does anything but call
  // `onClose`.
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent): void {
      if (event.key === 'Escape') onClose();
    }
    window.addEventListener('keydown', onKeyDown);
    return () => {
      window.removeEventListener('keydown', onKeyDown);
    };
  }, [onClose]);

  // Active for the dialog's whole lifetime: a process that exits on its own
  // while the user is still looking at a confirmation should close the
  // dialog too, not leave it asking about something already gone.
  useEffect(() => {
    let cancelled = false;

    const subscription = onProcessesTick((diff) => {
      if (cancelled) return;
      const gone = diff.removed.some(
        (key) => key.pid === target.key.pid && key.startedAt === target.key.startedAt
      );
      if (gone) onClose();
    });

    return () => {
      cancelled = true;
      subscription.then(
        (off) => {
          off();
        },
        () => {
          // Nothing to unsubscribe from if the subscription itself failed.
        }
      );
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target.key.pid, target.key.startedAt]);

  // The grace period: offers the forceful step once it elapses, unless the
  // subscription above has already closed the dialog. WAIT_CEILING_MS is a
  // second, longer timer to the same effect — a backstop so a stalled grace
  // timer cannot strand the user watching a spinner forever.
  useEffect(() => {
    if (state.phase !== 'waiting') return undefined;

    const grace = setTimeout(() => {
      setState({ phase: 'force' });
    }, GRACE_MS);
    const ceiling = setTimeout(() => {
      setState({ phase: 'force' });
    }, WAIT_CEILING_MS);

    return () => {
      clearTimeout(grace);
      clearTimeout(ceiling);
    };
  }, [state.phase]);

  function fail(error: unknown): void {
    setState({ phase: 'error', ...describeError(error) });
  }

  function acceptConfirm(): void {
    loadCriticalMatch(target).then((next) => {
      setState(next);
    }, fail);
  }

  function acceptCriticalConfirm(): void {
    setState({ phase: 'waiting' });
  }

  function requestGraceful(): void {
    terminateProcess(target.key, 'graceful').then((outcome) => {
      if (outcome === 'alreadyGone') {
        onClose();
      } else if (outcome === 'noWindowToClose') {
        // Nothing was asked to close, so there is nothing to wait five
        // seconds for — the forceful step is the only one that can do
        // anything here.
        setState({ phase: 'force' });
      }
      // 'signalled': stay in 'waiting'. The tick subscription or the grace
      // timer above decides what happens next.
    }, fail);
  }

  function requestForceful(): void {
    terminateProcess(target.key, 'forceful').then((outcome) => {
      if (outcome === 'alreadyGone') onClose();
      // 'signalled' is picked up by the tick subscription; there is nothing
      // else forceful can report.
    }, fail);
  }

  // The graceful request is sent the instant `waiting` is entered, whether
  // that follows a plain confirm or a critical one, so it is fired from an
  // effect rather than duplicated in both accept handlers.
  useEffect(() => {
    if (state.phase === 'waiting') requestGraceful();
    // Only the transition into 'waiting' should fire a request.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.phase]);

  const title = `End ${target.name}`;

  return (
    <div
      ref={dialogRef}
      role="dialog"
      aria-modal="true"
      aria-label={title}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
    >
      <div className="w-full max-w-sm rounded-xl border border-edge bg-surface-raised p-4 text-sm">
        <h2 className="text-sm font-medium">{title}</h2>

        {state.phase === 'confirm' && (
          <>
            <p className="mt-2 text-xs text-neutral-400">
              This ends {target.name} (PID {target.key.pid}). Any processes it started keep running
              on their own — ending it does not end them.
            </p>
            <DialogButtons onCancel={onClose} onAccept={acceptConfirm} acceptLabel={title} />
          </>
        )}

        {state.phase === 'criticalConfirm' && (
          <>
            <p role="alert" className="mt-2 text-xs text-amber-400">
              {target.name} is a system process. Ending it breaks {state.critical.breaks}.
            </p>
            <p className="mt-2 text-xs text-neutral-400">Are you sure you want to end it anyway?</p>
            <DialogButtons
              onCancel={onClose}
              onAccept={acceptCriticalConfirm}
              acceptLabel={`${title} anyway`}
            />
          </>
        )}

        {state.phase === 'waiting' && (
          <p role="status" className="mt-3 text-xs text-neutral-400">
            Waiting for {target.name} to exit…
          </p>
        )}

        {state.phase === 'force' && (
          <>
            <p className="mt-2 text-xs text-neutral-400">
              {target.name} has not exited. End it now, with no chance to save unsaved work?
            </p>
            <DialogButtons
              onCancel={onClose}
              onAccept={requestForceful}
              acceptLabel={`Force end ${target.name}`}
            />
          </>
        )}

        {state.phase === 'error' && (
          <>
            <p role="alert" className="mt-2 text-xs text-red-400">
              {state.identityMismatch
                ? `${target.name} has already ended.`
                : state.permissionDenied
                  ? `osstat cannot end ${target.name}: it belongs to another user.`
                  : `Could not end ${target.name}: ${state.message}`}
            </p>
            <div className="mt-3 flex justify-end">
              <button
                type="button"
                onClick={onClose}
                className="rounded-md border border-edge px-3 py-1 text-xs text-neutral-300 hover:bg-white/[0.04]"
              >
                Close
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

/** The cancel / accept button row shared by the confirmation phases. */
function DialogButtons({
  onCancel,
  onAccept,
  acceptLabel,
}: {
  onCancel: () => void;
  onAccept: () => void;
  acceptLabel: string;
}): React.JSX.Element {
  return (
    <div className="mt-3 flex justify-end gap-2">
      <button
        type="button"
        onClick={onCancel}
        className="rounded-md border border-edge px-3 py-1 text-xs text-neutral-300 hover:bg-white/[0.04]"
      >
        Cancel
      </button>
      <button
        type="button"
        onClick={onAccept}
        className="rounded-md border border-red-500/60 bg-red-500/10 px-3 py-1 text-xs text-red-300 hover:bg-red-500/20"
      >
        {acceptLabel}
      </button>
    </div>
  );
}
