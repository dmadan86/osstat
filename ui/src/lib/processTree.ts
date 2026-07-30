/**
 * Arranging flat process records into the tree the UI renders.
 *
 * This lives in the front-end rather than in `osstat-core` because the sampler
 * sends *diffs*: the consumer has to maintain its own structure to apply them
 * to, so a tree assembled in Rust would be rebuilt on arrival anyway, and
 * keeping both would mean two implementations of the same delicate
 * orphan-and-cycle handling drifting apart.
 *
 * Three invariants, each of which is a bug that would be easy to ship:
 *
 * 1. **A PID is not an identity.** Identity is PID *and* start time — see
 *    {@link keyOf}. Matching on PID alone shows a recycled PID as the same
 *    process inheriting the previous one's children.
 * 2. **Collapsing must never hide load.** A collapsed parent displays the sum
 *    of itself and every descendant, so the number on screen is the same
 *    whether a subtree is open or shut. Otherwise the tree lies by omission at
 *    exactly the moment someone is hunting for what is eating their machine.
 * 3. **The parent table is untrusted.** A snapshot can name a parent that has
 *    already exited or — read between two racing ticks — describe a cycle.
 *    Neither may hang, and neither may drop a process.
 */

import type { ProcessRecord } from '../bindings/ProcessRecord';

/** A process's own cost plus everything beneath it. */
export interface Rollup {
  /** Summed CPU percentage. */
  cpu: number;
  /** Summed resident memory in bytes. */
  memory: number;
  /** Summed read rate in bytes per second. */
  diskReadRate: number;
  /** Summed write rate in bytes per second. */
  diskWriteRate: number;
  /** How many processes are counted, including the one this describes. */
  processes: number;
}

/** A process and its descendants. */
export interface ProcessNode {
  /** Stable identity, usable as a React key. */
  key: string;
  /** This process. */
  record: ProcessRecord;
  /** Children, ordered by PID so the tree is stable between ticks. */
  children: ProcessNode[];
  /** This process plus everything beneath it. */
  rollup: Rollup;
}

/** A tree, plus the total across every process in it. */
export interface ProcessTree {
  /** Processes with no live parent, ordered by PID. */
  roots: ProcessNode[];
  /** Every process on the machine, summed. */
  total: Rollup;
}

/** One rendered row: a node plus where it sits. */
export interface ProcessRow {
  /** The node this row shows. */
  node: ProcessNode;
  /** Indentation level, zero for a root. */
  depth: number;
  /** Whether this node has children at all. */
  expandable: boolean;
  /** Whether its children are currently shown. */
  expanded: boolean;
}

/** Which column the tree is ordered by. */
export type SortColumn = 'name' | 'pid' | 'user' | 'cpu' | 'memory' | 'disk';

/** How a column is ordered. */
export type SortDirection = 'asc' | 'desc';

/** A complete ordering. */
export interface Sort {
  /** The column to order by. */
  column: SortColumn;
  /** The direction to order in. */
  direction: SortDirection;
}

/**
 * The identity of a process, as a string usable for map and React keys.
 *
 * @param record The process.
 * @returns A key combining PID and start time.
 */
export function keyOf(record: Pick<ProcessRecord, 'pid' | 'startedAt'>): string {
  return `${record.pid}:${record.startedAt}`;
}

/** An empty roll-up. */
function emptyRollup(): Rollup {
  return { cpu: 0, memory: 0, diskReadRate: 0, diskWriteRate: 0, processes: 0 };
}

/** Folds `addition` into `into`. */
function absorb(into: Rollup, addition: Rollup): void {
  into.cpu += addition.cpu;
  into.memory += addition.memory;
  into.diskReadRate += addition.diskReadRate;
  into.diskWriteRate += addition.diskWriteRate;
  into.processes += addition.processes;
}

/**
 * Arranges flat records into a tree, with roll-ups computed.
 *
 * Every input record appears exactly once in the output, whatever the parent
 * table claims: an unknown parent, a self-parenting root such as PID 0, and a
 * cycle all resolve by promoting a process to a root rather than by dropping
 * it.
 *
 * @param records The processes to arrange. Order does not matter.
 * @returns The roots and the total across everything.
 */
export function buildTree(records: readonly ProcessRecord[]): ProcessTree {
  // Sorting by PID first makes every later step deterministic, which is what
  // lets cycle-breaking produce the same tree from the same snapshot.
  const byPid = new Map<number, ProcessRecord>();
  for (const record of [...records].sort((a, b) => a.pid - b.pid)) {
    if (!byPid.has(record.pid)) byPid.set(record.pid, record);
  }

  const nodes = new Map<number, ProcessNode>();
  for (const [pid, record] of byPid) {
    nodes.set(pid, {
      key: keyOf(record),
      record,
      children: [],
      rollup: {
        cpu: record.cpu,
        memory: record.memory,
        diskReadRate: record.diskReadRate,
        diskWriteRate: record.diskWriteRate,
        processes: 1,
      },
    });
  }

  const parentOf = new Map<number, number>();
  for (const [pid, record] of byPid) {
    const parent = record.parentPid;
    if (parent === null || parent === pid || !byPid.has(parent)) continue;
    parentOf.set(pid, parent);
  }

  breakCycles(parentOf);

  const roots: ProcessNode[] = [];
  for (const [pid, node] of nodes) {
    const parent = parentOf.get(pid);
    if (parent === undefined) {
      roots.push(node);
    } else {
      nodes.get(parent)?.children.push(node);
    }
  }

  const total = emptyRollup();
  for (const root of roots) {
    accumulate(root);
    absorb(total, root.rollup);
  }

  return { roots, total };
}

/**
 * Removes one edge from every parent cycle.
 *
 * Walks each chain of ancestors once, marking nodes as it goes. Re-entering a
 * node still on the current walk means the walk has closed a loop, and that
 * node's parent edge is the one cut.
 *
 * @param parentOf Child PID to parent PID; mutated in place.
 */
function breakCycles(parentOf: Map<number, number>): void {
  const WALKING = 1;
  const SETTLED = 2;
  const state = new Map<number, number>();

  for (const start of [...parentOf.keys()].sort((a, b) => a - b)) {
    if (state.has(start)) continue;

    const walked: number[] = [];
    let current: number | undefined = start;

    while (current !== undefined) {
      const mark = state.get(current);
      if (mark === WALKING) {
        parentOf.delete(current);
        break;
      }
      if (mark === SETTLED) break;

      state.set(current, WALKING);
      walked.push(current);
      current = parentOf.get(current);
    }

    for (const pid of walked) state.set(pid, SETTLED);
  }
}

/**
 * Computes roll-ups for a subtree and orders its children by PID.
 *
 * Iterative rather than recursive: a process chain can in principle be
 * thousands deep, and a stack overflow in a system monitor is not an acceptable
 * failure mode.
 *
 * @param root The subtree to process, mutated in place.
 */
function accumulate(root: ProcessNode): void {
  const order: ProcessNode[] = [];
  const stack: ProcessNode[] = [root];

  while (stack.length > 0) {
    const node = stack.pop();
    if (node === undefined) break;
    node.children.sort((a, b) => a.record.pid - b.record.pid);
    order.push(node);
    stack.push(...node.children);
  }

  // Deepest first, so a parent folds in children that are already complete.
  for (let index = order.length - 1; index >= 0; index -= 1) {
    const node = order[index];
    if (node === undefined) continue;
    for (const child of node.children) absorb(node.rollup, child.rollup);
  }
}

/** Reads the value a column sorts on. */
function sortValue(node: ProcessNode, column: SortColumn): number | string {
  switch (column) {
    case 'name':
      return node.record.name.toLowerCase();
    case 'pid':
      return node.record.pid;
    case 'user':
      return (node.record.user ?? '').toLowerCase();
    case 'cpu':
      return node.rollup.cpu;
    case 'memory':
      return node.rollup.memory;
    case 'disk':
      return node.rollup.diskReadRate + node.rollup.diskWriteRate;
  }
}

/** Orders siblings by the chosen column. */
function compare(a: ProcessNode, b: ProcessNode, sort: Sort): number {
  const left = sortValue(a, sort.column);
  const right = sortValue(b, sort.column);

  let result: number;
  if (typeof left === 'string' && typeof right === 'string') {
    result = left.localeCompare(right);
  } else {
    result = Number(left) - Number(right);
  }

  // PID as a tiebreak, so equal values do not shuffle between ticks.
  if (result === 0) result = a.record.pid - b.record.pid;
  return sort.direction === 'asc' ? result : -result;
}

/**
 * Whether a record matches a search term.
 *
 * @param record The process to test.
 * @param needle A lowercased search term.
 */
function matches(record: ProcessRecord, needle: string): boolean {
  return (
    record.name.toLowerCase().includes(needle) ||
    String(record.pid).includes(needle) ||
    (record.user ?? '').toLowerCase().includes(needle)
  );
}

/**
 * Keeps only the subtrees containing a match, with roll-ups left intact.
 *
 * A parent is kept when a descendant matches, because hiding the branch would
 * hide the match. Its roll-up still describes the whole original subtree — a
 * filtered view answers "where is this process" and must not double as a
 * different set of totals.
 *
 * @param nodes The nodes to filter.
 * @param needle A lowercased search term.
 * @returns Nodes on a path to a match.
 */
function filterTree(nodes: readonly ProcessNode[], needle: string): ProcessNode[] {
  const kept: ProcessNode[] = [];

  for (const node of nodes) {
    const children = filterTree(node.children, needle);
    if (children.length > 0 || matches(node.record, needle)) {
      kept.push({ ...node, children });
    }
  }

  return kept;
}

/** Options controlling how a tree becomes a list of rows. */
export interface FlattenOptions {
  /** Keys of the nodes whose children are shown. */
  expanded: ReadonlySet<string>;
  /** Search term; empty shows everything. */
  search?: string;
  /** Column ordering. */
  sort?: Sort;
  /** Ignore parentage and list every process at depth zero. */
  flat?: boolean;
}

/**
 * Turns a tree into the ordered list of rows to render.
 *
 * When a search is active, every surviving branch is expanded regardless of
 * {@link FlattenOptions.expanded}: a filter that leaves its matches hidden
 * inside collapsed parents is worse than no filter, because it reports finding
 * something and then does not show it.
 *
 * @param tree The tree to flatten.
 * @param options How to filter, order and expand.
 * @returns Rows in display order.
 */
export function flattenTree(tree: ProcessTree, options: FlattenOptions): ProcessRow[] {
  const needle = (options.search ?? '').trim().toLowerCase();
  const searching = needle.length > 0;
  const sort = options.sort ?? { column: 'cpu', direction: 'desc' };

  const roots = searching ? filterTree(tree.roots, needle) : tree.roots;

  if (options.flat === true) {
    const everything: ProcessNode[] = [];
    const stack = [...roots];
    while (stack.length > 0) {
      const node = stack.pop();
      if (node === undefined) break;
      everything.push(node);
      stack.push(...node.children);
    }
    return everything
      .sort((a, b) => compare(a, b, sort))
      .map((node) => ({ node, depth: 0, expandable: false, expanded: false }));
  }

  const rows: ProcessRow[] = [];

  const visit = (nodes: readonly ProcessNode[], depth: number): void => {
    for (const node of [...nodes].sort((a, b) => compare(a, b, sort))) {
      const expandable = node.children.length > 0;
      const expanded = expandable && (searching || options.expanded.has(node.key));
      rows.push({ node, depth, expandable, expanded });
      if (expanded) visit(node.children, depth + 1);
    }
  };

  visit(roots, 0);
  return rows;
}

/**
 * Flattens a tree into a PID-keyed lookup.
 *
 * For a caller that only has a bare PID to work from — the port table,
 * notably, which has no start time of its own and must resolve a real
 * {@link ProcessRecord} from the tree before it can build a `ProcessKey`
 * safe to end.
 *
 * @param tree The tree to flatten.
 * @returns Every process in the tree, keyed by PID.
 */
export function recordsByPid(tree: ProcessTree): Map<number, ProcessRecord> {
  const byPid = new Map<number, ProcessRecord>();
  const stack = [...tree.roots];

  while (stack.length > 0) {
    const node = stack.pop();
    if (node === undefined) break;
    byPid.set(node.record.pid, node.record);
    stack.push(...node.children);
  }

  return byPid;
}

/**
 * Applies a tick's changes to the records the UI holds.
 *
 * @param current The records held, keyed by {@link keyOf}.
 * @param diff What the sampler reported.
 * @returns A new map; the input is not modified.
 */
export function applyDiff(
  current: ReadonlyMap<string, ProcessRecord>,
  diff: {
    added: readonly ProcessRecord[];
    removed: readonly { pid: number; startedAt: number }[];
    changed: readonly ProcessRecord[];
  }
): Map<string, ProcessRecord> {
  const next = new Map(current);

  for (const key of diff.removed) next.delete(keyOf(key));
  for (const record of diff.added) next.set(keyOf(record), record);
  for (const record of diff.changed) next.set(keyOf(record), record);

  return next;
}
