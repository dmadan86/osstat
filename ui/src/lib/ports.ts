/**
 * Filtering and ordering the port table.
 *
 * Kept apart from `Ports.tsx` for the same reason `processTree.ts` is kept
 * apart from `Processes.tsx`: it is the part cheap to test exhaustively
 * without a socket open or a component mounted.
 */

import type { PortRecord } from '../bindings/PortRecord';

/** What the port filter narrows on. Both halves apply together (`AND`). */
export interface PortFilter {
  /** Matched against local and remote port numbers, as a substring. */
  port?: string;
  /** Matched against process name and PID, case-insensitively. */
  process?: string;
}

/**
 * Whether a row's port matches the port half of the filter.
 *
 * @param row The row to test.
 * @param needle The port filter text; blank matches everything.
 */
function matchesPort(row: PortRecord, needle: string | undefined): boolean {
  const trimmed = (needle ?? '').trim();
  if (trimmed.length === 0) return true;

  return (
    String(row.localPort).includes(trimmed) ||
    (row.remotePort !== null && String(row.remotePort).includes(trimmed))
  );
}

/**
 * Whether a row's process matches the process half of the filter.
 *
 * @param row The row to test.
 * @param needle The process filter text; blank matches everything.
 */
function matchesProcess(row: PortRecord, needle: string | undefined): boolean {
  const trimmed = (needle ?? '').trim().toLowerCase();
  if (trimmed.length === 0) return true;

  return (
    (row.processName ?? '').toLowerCase().includes(trimmed) ||
    (row.pid !== null && String(row.pid).includes(trimmed))
  );
}

/**
 * Whether a row survives both halves of the filter.
 *
 * @param row The row to test.
 * @param filter The filter to apply.
 */
export function matchesFilter(row: PortRecord, filter: PortFilter): boolean {
  return matchesPort(row, filter.port) && matchesProcess(row, filter.process);
}

/**
 * Filters and orders rows for display.
 *
 * Listening sockets sort first, then by local port: that is the order a user
 * hunting for "what's listening on this machine" wants, rather than whatever
 * order the platform's socket table happened to enumerate in.
 *
 * @param rows The rows to filter.
 * @param filter The filter to apply.
 * @returns A new, sorted array; the input is not modified.
 */
export function filterPorts(rows: readonly PortRecord[], filter: PortFilter): PortRecord[] {
  return rows
    .filter((row) => matchesFilter(row, filter))
    .slice()
    .sort((a, b) => {
      const listening = Number(b.state === 'listen') - Number(a.state === 'listen');
      if (listening !== 0) return listening;
      if (a.localPort !== b.localPort) return a.localPort - b.localPort;
      return (a.processName ?? '').localeCompare(b.processName ?? '');
    });
}
