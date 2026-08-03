/**
 * The port inspector.
 *
 * Ending the owning process shares the same confirmation dialog and
 * identity-checked backend as the process tree — see `EndProcessDialog` and
 * `crates/osstat-platform/src/terminate.rs`. `PortRecord` carries a PID and
 * no start time (see "Why the join is by PID alone" in
 * `crates/osstat-core/src/socket.rs`), which is a fine trade for *display*
 * but not for *ending* a process: a real `ProcessKey` is resolved from the
 * process tree by PID before the dialog opens, and the button is disabled
 * when that PID is not currently in the tree rather than sent with a
 * synthesised start time, which would either fail identity on every attempt
 * or, worse, match whatever genuinely started at the epoch.
 *
 * Unlike the process table this is not pushed from a background tick — see
 * `src-tauri/src/ports.rs` for why — so this page fetches on mount and again
 * whenever the user asks, rather than following an event stream.
 */

import { useEffect, useMemo, useState } from 'react';

import type { PortRecord } from '../bindings/PortRecord';
import type { ProcessRecord } from '../bindings/ProcessRecord';
import { fetchPortList } from '../lib/ipc';
import { filterPorts } from '../lib/ports';
import { recordsByPid, type ProcessTree } from '../lib/processTree';
import type { EndProcessTarget } from '../lib/termination';

/** What a load can be doing. */
type LoadState =
  | { status: 'loading' }
  | { status: 'ready'; rows: PortRecord[] }
  | { status: 'error'; message: string };

/** The columns, in display order. */
const COLUMNS = ['Protocol', 'Local', 'Remote', 'State', 'PID', 'Process', 'Path', 'End'] as const;

/** Renders an unknown thrown value as a message. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** A stable empty array, so a page with nothing loaded yet does not hand
 * `useMemo` a fresh `[]` reference every render. */
const NO_ROWS: PortRecord[] = [];

/** What the Ports page needs. */
export interface PortsProps {
  /** The current process tree, used only to resolve an owning process's real `ProcessKey` by PID before ending it. */
  tree: ProcessTree;
  /** Opens the confirmation dialog for a process the user wants ended. */
  onEndProcess: (target: EndProcessTarget) => void;
}

/** Renders the port inspector. */
export function Ports({ tree, onEndProcess }: PortsProps): React.JSX.Element {
  const [state, setState] = useState<LoadState>({ status: 'loading' });
  const [port, setPort] = useState('');
  const [processFilter, setProcessFilter] = useState('');

  const byPid = useMemo(() => recordsByPid(tree), [tree]);

  // Load once on mount, the same cancellation-guarded shape as
  // `useSystemDescription` in `lib/useLiveData.ts`. There is no push event to
  // follow — see the module docs — so a stale table thereafter is refreshed
  // by the button below, not silently.
  useEffect(() => {
    let cancelled = false;

    fetchPortList().then(
      (rows) => {
        if (!cancelled) setState({ status: 'ready', rows });
      },
      (error: unknown) => {
        if (!cancelled) setState({ status: 'error', message: messageOf(error) });
      }
    );

    return () => {
      cancelled = true;
    };
  }, []);

  // Unlike the mount effect, this runs from a click, not from React's render
  // phase, so setting `loading` synchronously here is not the anti-pattern
  // it would be inside `useEffect`.
  const refresh = (): void => {
    setState((current) => (current.status === 'ready' ? current : { status: 'loading' }));
    fetchPortList().then(
      (rows) => {
        setState({ status: 'ready', rows });
      },
      (error: unknown) => {
        setState({ status: 'error', message: messageOf(error) });
      }
    );
  };

  const rows = state.status === 'ready' ? state.rows : NO_ROWS;
  const filtered = useMemo(
    () => filterPorts(rows, { port, process: processFilter }),
    [rows, port, processFilter]
  );

  return (
    <div className="flex h-full flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <input
          type="search"
          value={port}
          onChange={(event) => {
            setPort(event.target.value);
          }}
          placeholder="Filter by port…"
          aria-label="Filter by port"
          className="w-36 rounded-md border border-edge bg-black/20 px-3 py-1.5 text-sm outline-none placeholder:text-text-faint focus:border-accent"
        />
        <input
          type="search"
          value={processFilter}
          onChange={(event) => {
            setProcessFilter(event.target.value);
          }}
          placeholder="Filter by process or PID…"
          aria-label="Filter by process"
          className="min-w-48 flex-1 rounded-md border border-edge bg-black/20 px-3 py-1.5 text-sm outline-none placeholder:text-text-faint focus:border-accent"
        />

        <button
          type="button"
          onClick={refresh}
          className="rounded-md border border-edge px-3 py-1.5 text-xs text-text hover:bg-white/[0.04]"
        >
          Refresh
        </button>
      </div>

      <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-edge bg-surface-raised">
        <div
          role="row"
          className="flex shrink-0 items-center gap-3 border-b border-edge px-3 py-1.5 text-[10px] uppercase tracking-wider text-text-muted"
        >
          {COLUMNS.map((label) => (
            <span
              key={label}
              className={
                label === 'Process' || label === 'Path'
                  ? 'min-w-0 flex-1'
                  : label === 'PID'
                    ? 'w-16 text-right'
                    : label === 'End'
                      ? 'w-16'
                      : 'w-32'
              }
            >
              {label}
            </span>
          ))}
        </div>

        {state.status === 'error' && (
          <p role="alert" className="p-6 text-center text-sm text-red-400">
            Could not read the port table: {state.message}
          </p>
        )}

        {state.status === 'loading' && (
          <p role="status" className="p-6 text-center text-sm text-text-muted">
            Reading the port table…
          </p>
        )}

        {state.status === 'ready' && filtered.length === 0 && (
          <p role="status" className="p-6 text-center text-sm text-text-muted">
            No socket matches this filter.
          </p>
        )}

        {state.status === 'ready' && filtered.length > 0 && (
          <div className="min-h-0 flex-1 overflow-auto">
            {filtered.map((row, index) => (
              <Row
                // A socket has no identity of its own besides what it shows,
                // and two rows can legitimately be identical (a PID with two
                // sockets on the same port during a fast rebind); the index
                // breaks that tie for React's sake without claiming a fake
                // uniqueness the data does not have.
                key={`${row.protocol}-${row.localAddress}-${row.localPort}-${row.pid ?? 'none'}-${index}`}
                row={row}
                owner={row.pid !== null ? byPid.get(row.pid) : undefined}
                onEndProcess={onEndProcess}
              />
            ))}
          </div>
        )}

        <div className="flex shrink-0 items-center justify-between gap-3 border-t border-edge px-3 py-1.5 text-[11px] text-text-muted">
          <span>
            {rows.length} sockets
            {filtered.length !== rows.length && ` · ${filtered.length} matching`}
          </span>
        </div>
      </div>
    </div>
  );
}

/** One rendered socket row. */
function Row({
  row,
  owner,
  onEndProcess,
}: {
  row: PortRecord;
  /** The owning process, resolved from the tree by PID. `undefined` when the
   * PID is absent or is not currently in the tree — the End control disables
   * itself rather than guessing. */
  owner: ProcessRecord | undefined;
  onEndProcess: (target: EndProcessTarget) => void;
}): React.JSX.Element {
  const listening = row.state === 'listen';

  return (
    <div
      role="row"
      className={`flex items-center gap-3 px-3 py-1 text-xs hover:bg-white/[0.035] ${
        listening ? 'bg-accent/10' : ''
      }`}
    >
      <span className="w-32 uppercase text-text-muted">
        {row.protocol}
        {listening && (
          <span className="ml-1.5 rounded-full border border-accent px-1.5 text-[10px] normal-case text-accent">
            listening
          </span>
        )}
      </span>
      <span data-selectable className="w-32 truncate font-mono">
        {row.localAddress}:{row.localPort}
      </span>
      <span data-selectable className="w-32 truncate font-mono text-text-muted">
        {row.remoteAddress !== null ? `${row.remoteAddress}:${row.remotePort}` : '—'}
      </span>
      <span className="w-32 text-text-muted">{row.state ?? '—'}</span>
      <span data-selectable className="w-16 text-right font-mono text-text-muted">
        {row.pid ?? '—'}
      </span>
      <span data-selectable className="min-w-0 flex-1 truncate">
        {row.processName ?? '—'}
      </span>
      <span
        data-selectable
        className="min-w-0 flex-1 truncate text-text-muted"
        title={row.processPath ?? undefined}
      >
        {row.processPath ?? '—'}
      </span>
      <span className="w-16 shrink-0">
        <button
          type="button"
          disabled={owner === undefined}
          aria-label={owner !== undefined ? `End ${owner.name}` : 'End owning process'}
          title={owner === undefined ? 'The owning process could not be identified.' : undefined}
          onClick={() => {
            if (owner === undefined) return;
            onEndProcess({
              key: { pid: owner.pid, startedAt: owner.startedAt },
              name: owner.name,
              exe: owner.exe,
            });
          }}
          className="rounded-md border border-edge px-2 py-0.5 text-[10px] text-text-muted hover:bg-white/[0.04] disabled:cursor-not-allowed disabled:opacity-40"
        >
          End
        </button>
      </span>
    </div>
  );
}
