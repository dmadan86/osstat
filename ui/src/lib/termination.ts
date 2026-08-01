/**
 * The decisions behind ending a process, separated from the dialog that shows
 * them.
 *
 * Pure so the rules can be tested without rendering: which confirmations a
 * target needs, and how long to wait before offering the forceful step.
 */

import type { CriticalProcess } from '../bindings/CriticalProcess';

/** A process the user has asked to end. */
export interface EndProcessTarget {
  /** PID and start time. Never a bare PID — see `terminateProcess`. */
  key: { pid: number; startedAt: number };
  /** Executable name, for the confirmation. */
  name: string;
  /** Full path, where it could be read. `null` is never a critical match. */
  exe: string | null;
}

/**
 * How long to wait for a graceful exit before offering the forceful step.
 *
 * The process stream ticks every 2 s and slows when the window is not in the
 * foreground, so detection lands somewhere past 5 s rather than at it. The
 * ceiling exists so a paused sampler cannot strand the user in a spinner.
 */
export const WAIT_CEILING_MS = 15_000;

/** How long before the forceful step is offered, when the stream is healthy. */
export const GRACE_MS = 5_000;

/**
 * Finds the critical-list entry matching a target, if any.
 *
 * Mirrors `CriticalList::match_for` in Rust: name and path must both match, and
 * a null path never matches. Duplicated rather than fetched per-target because
 * the list is small, static, and the dialog must decide synchronously.
 */
export function matchCritical(
  list: readonly CriticalProcess[],
  name: string,
  exe: string | null
): CriticalProcess | null {
  if (exe === null) return null;

  const normalised = exe.replaceAll('\\', '/').toLowerCase();

  return (
    list.find(
      (process) =>
        process.name.toLowerCase() === name.toLowerCase() &&
        process.paths.some((path) => normalised.endsWith(path.replaceAll('\\', '/').toLowerCase()))
    ) ?? null
  );
}
