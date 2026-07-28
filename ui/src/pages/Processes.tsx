/**
 * The process tree.
 *
 * Read-only in this phase. There is deliberately no kill affordance at all —
 * not even a disabled one, which would promise something this phase does not
 * deliver and invite a bug report instead of setting an expectation.
 */

import { useMemo, useRef, useState } from 'react';

import { INK, SEQUENTIAL } from '../charts/palette';
import { formatBytes, formatPercent, formatRate } from '../lib/format';
import {
  flattenTree,
  type ProcessRow,
  type ProcessTree,
  type Sort,
  type SortColumn,
} from '../lib/processTree';

/** What the Processes page needs. */
export interface ProcessesProps {
  /** The current tree. */
  tree: ProcessTree;
  /** Whether the first snapshot has arrived. */
  loaded: boolean;
}

/** Height of one row in pixels; the windowing maths depends on it being fixed. */
const ROW_HEIGHT = 26;

/** Rows rendered beyond the viewport, so a fast scroll does not show blanks. */
const OVERSCAN = 8;

/** The columns, in display order. */
const COLUMNS: { id: SortColumn; label: string; width: string; numeric: boolean }[] = [
  { id: 'name', label: 'Name', width: 'flex-1 min-w-0', numeric: false },
  { id: 'pid', label: 'PID', width: 'w-16', numeric: true },
  { id: 'user', label: 'User', width: 'w-24', numeric: false },
  { id: 'cpu', label: 'CPU', width: 'w-24', numeric: true },
  { id: 'memory', label: 'Memory', width: 'w-24', numeric: true },
  { id: 'disk', label: 'Disk', width: 'w-20', numeric: true },
];

/**
 * Renders the process tree.
 *
 * @param props The tree and whether it has loaded.
 */
export function Processes({ tree, loaded }: ProcessesProps): React.JSX.Element {
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const [search, setSearch] = useState('');
  const [flat, setFlat] = useState(false);
  const [sort, setSort] = useState<Sort>({ column: 'cpu', direction: 'desc' });
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(600);
  const viewport = useRef<HTMLDivElement | null>(null);

  const rows = useMemo(
    () => flattenTree(tree, { expanded, search, sort, flat }),
    [tree, expanded, search, sort, flat]
  );

  // Hand-rolled windowing. One list does not justify a virtualisation
  // dependency, and the flattening step it needs exists anyway for
  // expand/collapse.
  const first = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
  const visibleCount = Math.ceil(viewportHeight / ROW_HEIGHT) + OVERSCAN * 2;
  const visible = rows.slice(first, first + visibleCount);

  const toggle = (key: string): void => {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const sortBy = (column: SortColumn): void => {
    setSort((current) =>
      current.column === column
        ? { column, direction: current.direction === 'asc' ? 'desc' : 'asc' }
        : { column, direction: column === 'name' || column === 'user' ? 'asc' : 'desc' }
    );
  };

  const matching = search.trim().length > 0 ? rows.length : null;

  return (
    <div className="flex h-full flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <input
          type="search"
          value={search}
          onChange={(event) => {
            setSearch(event.target.value);
          }}
          placeholder="Filter by name, PID or user…"
          aria-label="Filter processes"
          className="min-w-48 flex-1 rounded-md border border-edge bg-black/20 px-3 py-1.5 text-sm outline-none placeholder:text-neutral-600 focus:border-accent"
        />

        <button
          type="button"
          onClick={() => {
            setFlat((current) => !current);
          }}
          aria-pressed={flat}
          className="rounded-md border border-edge px-3 py-1.5 text-xs text-neutral-300 hover:bg-white/[0.04]"
        >
          {flat ? 'Flat list' : 'Tree'}
        </button>
      </div>

      <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-edge bg-surface-raised">
        <div
          role="row"
          className="flex shrink-0 items-center gap-3 border-b border-edge px-3 py-1.5 text-[10px] uppercase tracking-wider text-neutral-500"
        >
          {COLUMNS.map((column) => (
            <button
              key={column.id}
              type="button"
              role="columnheader"
              aria-sort={
                sort.column === column.id
                  ? sort.direction === 'asc'
                    ? 'ascending'
                    : 'descending'
                  : 'none'
              }
              onClick={() => {
                sortBy(column.id);
              }}
              className={`${column.width} ${column.numeric ? 'text-right' : 'text-left'} hover:text-neutral-300 ${
                sort.column === column.id ? 'text-accent' : ''
              }`}
            >
              {column.label}
              {sort.column === column.id && (
                <span aria-hidden="true">{sort.direction === 'asc' ? ' ↑' : ' ↓'}</span>
              )}
            </button>
          ))}
        </div>

        {!loaded ? (
          <p role="status" className="p-6 text-center text-sm text-neutral-500">
            Reading the process table…
          </p>
        ) : rows.length === 0 ? (
          <p role="status" className="p-6 text-center text-sm text-neutral-500">
            No process matches “{search}”.
          </p>
        ) : (
          <div
            ref={viewport}
            onScroll={(event) => {
              setScrollTop(event.currentTarget.scrollTop);
              setViewportHeight(event.currentTarget.clientHeight);
            }}
            className="min-h-0 flex-1 overflow-auto"
          >
            {/* A spacer of the full height keeps the scrollbar honest while
                only the visible slice is in the DOM. */}
            <div style={{ height: rows.length * ROW_HEIGHT, position: 'relative' }}>
              <div style={{ transform: `translateY(${first * ROW_HEIGHT}px)` }}>
                {visible.map((row) => (
                  <Row key={row.node.key} row={row} onToggle={toggle} />
                ))}
              </div>
            </div>
          </div>
        )}

        <div className="flex shrink-0 items-center justify-between gap-3 border-t border-edge px-3 py-1.5 text-[11px] text-neutral-500">
          <span>
            {tree.total.processes} processes
            {matching !== null && ` · ${matching} matching`}
          </span>
          <span>
            {formatPercent(tree.total.cpu)} CPU · {formatBytes(tree.total.memory)}
          </span>
        </div>
      </div>
    </div>
  );
}

/** One rendered process row. */
function Row({
  row,
  onToggle,
}: {
  row: ProcessRow;
  onToggle: (key: string) => void;
}): React.JSX.Element {
  const { node, depth, expandable, expanded } = row;
  // The roll-up, not the process's own figure: a collapsed parent must account
  // for its whole subtree or collapsing would make load disappear.
  const io = node.rollup.diskReadRate + node.rollup.diskWriteRate;

  return (
    <div
      role="row"
      style={{ height: ROW_HEIGHT }}
      className="flex items-center gap-3 px-3 text-xs hover:bg-white/[0.035]"
    >
      <span className="flex min-w-0 flex-1 items-center gap-1.5">
        <span style={{ width: depth * 14 }} aria-hidden="true" />
        {expandable ? (
          <button
            type="button"
            aria-expanded={expanded}
            aria-label={`${expanded ? 'Collapse' : 'Expand'} ${node.record.name}`}
            onClick={() => {
              onToggle(node.key);
            }}
            className="w-3 shrink-0 text-accent"
          >
            {expanded ? '▾' : '▸'}
          </button>
        ) : (
          <span className="w-3 shrink-0 text-center text-neutral-700" aria-hidden="true">
            ·
          </span>
        )}

        <span data-selectable className="truncate" title={node.record.exe ?? node.record.name}>
          {node.record.name}
        </span>

        {expandable && !expanded && (
          <span className="shrink-0 rounded-full border border-edge px-1.5 text-[10px] text-neutral-500">
            {node.rollup.processes - 1}
          </span>
        )}

        {node.record.status === 'zombie' && (
          <span className="shrink-0 text-[10px] uppercase text-amber-500">zombie</span>
        )}
      </span>

      <span data-selectable className="w-16 text-right font-mono text-neutral-500">
        {node.record.pid}
      </span>
      <span className="w-24 truncate text-neutral-500">{node.record.user ?? '—'}</span>
      <span className="flex w-24 shrink-0 items-center justify-end gap-1.5">
        {/* An inline meter rather than the full component: at 26px a row has no
            space for a label line, and the number beside it is the label. */}
        <span
          aria-hidden="true"
          className="h-1 w-10 overflow-hidden rounded-full"
          style={{ backgroundColor: INK.gridline }}
        >
          <span
            className="block h-full rounded-full"
            style={{
              width: `${Math.min(100, Math.max(0, node.rollup.cpu))}%`,
              backgroundColor: SEQUENTIAL[3],
            }}
          />
        </span>
        <span data-selectable className="font-mono">
          {formatPercent(node.rollup.cpu)}
        </span>
      </span>
      <span data-selectable className="w-24 text-right font-mono">
        {formatBytes(node.rollup.memory)}
      </span>
      <span data-selectable className="w-20 text-right font-mono text-neutral-500">
        {io > 0 ? formatRate(io) : '—'}
      </span>
    </div>
  );
}
