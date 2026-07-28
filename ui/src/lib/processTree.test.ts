import { describe, expect, it } from 'vitest';

import type { ProcessRecord } from '../bindings/ProcessRecord';
import { applyDiff, buildTree, flattenTree, keyOf, type ProcessNode } from './processTree';

function record(pid: number, parentPid: number | null, overrides: Partial<ProcessRecord> = {}) {
  return {
    pid,
    parentPid,
    name: `proc-${pid}`,
    exe: null,
    user: 'tester',
    cpu: 1,
    memory: 1024,
    diskReadRate: 0,
    diskWriteRate: 0,
    startedAt: 1000,
    status: 'running',
    ...overrides,
  } satisfies ProcessRecord;
}

/** Every PID in a tree, however deep. */
function allPids(nodes: readonly ProcessNode[]): number[] {
  const found: number[] = [];
  const stack = [...nodes];
  while (stack.length > 0) {
    const node = stack.pop();
    if (node === undefined) break;
    found.push(node.record.pid);
    stack.push(...node.children);
  }
  return found.sort((a, b) => a - b);
}

describe('keyOf', () => {
  it('distinguishes a recycled PID from the process that had it', () => {
    expect(keyOf({ pid: 42, startedAt: 1000 })).not.toBe(keyOf({ pid: 42, startedAt: 2000 }));
  });
});

describe('buildTree', () => {
  it('builds an empty tree from no records', () => {
    const tree = buildTree([]);

    expect(tree.roots).toEqual([]);
    expect(tree.total.processes).toBe(0);
  });

  it('hangs children off their parent', () => {
    const tree = buildTree([record(1, null), record(2, 1), record(3, 1), record(4, 2)]);

    expect(tree.roots).toHaveLength(1);
    expect(tree.roots[0]?.record.pid).toBe(1);
    expect(tree.roots[0]?.children.map((child) => child.record.pid)).toEqual([2, 3]);
    expect(tree.roots[0]?.children[0]?.children[0]?.record.pid).toBe(4);
  });

  it('orders children by PID so the tree is stable between ticks', () => {
    const tree = buildTree([record(1, null), record(9, 1), record(3, 1), record(7, 1)]);

    expect(tree.roots[0]?.children.map((child) => child.record.pid)).toEqual([3, 7, 9]);
  });

  it('promotes an orphan to a root rather than dropping it', () => {
    // The parent exited between the two reads that produced this snapshot.
    const tree = buildTree([record(1, null), record(2, 404)]);

    expect(tree.total.processes).toBe(2);
    expect(allPids(tree.roots)).toEqual([1, 2]);
  });

  it('promotes a self-parenting process to a root', () => {
    // PID 0 names itself as its own parent on more than one platform.
    const tree = buildTree([record(0, 0), record(1, 0)]);

    expect(tree.roots).toHaveLength(1);
    expect(tree.roots[0]?.record.pid).toBe(0);
    expect(tree.total.processes).toBe(2);
  });

  it('breaks a two-process cycle without losing either process', () => {
    const tree = buildTree([record(1, 2), record(2, 1)]);

    expect(tree.total.processes).toBe(2);
    expect(allPids(tree.roots)).toEqual([1, 2]);
  });

  it('breaks a long cycle and keeps every process', () => {
    const tree = buildTree([record(1, 4), record(2, 1), record(3, 2), record(4, 3)]);

    expect(tree.total.processes).toBe(4);
    expect(allPids(tree.roots)).toEqual([1, 2, 3, 4]);
  });

  it('breaks cycles deterministically', () => {
    const build = () => buildTree([record(5, 7), record(7, 9), record(9, 5)]);

    expect(allPids(build().roots)).toEqual(allPids(build().roots));
    expect(build().roots.map((r) => r.record.pid)).toEqual(build().roots.map((r) => r.record.pid));
  });

  it('counts a duplicate PID once', () => {
    expect(buildTree([record(1, null), record(1, null)]).total.processes).toBe(1);
  });

  it('handles a chain thousands deep without overflowing the stack', () => {
    const records = [record(1, null)];
    for (let pid = 2; pid < 5000; pid += 1) records.push(record(pid, pid - 1));

    expect(buildTree(records).total.processes).toBe(4999);
  });

  it('makes a collapsed parent report the cost of its whole subtree', () => {
    // The invariant that makes collapsing safe: the number on screen is the
    // same whether the subtree is open or shut.
    const tree = buildTree([
      record(1, null, { cpu: 1, memory: 100 }),
      record(2, 1, { cpu: 2, memory: 200 }),
      record(3, 2, { cpu: 4, memory: 400 }),
    ]);

    expect(tree.roots[0]?.rollup.cpu).toBeCloseTo(7);
    expect(tree.roots[0]?.rollup.memory).toBe(700);
    expect(tree.roots[0]?.rollup.processes).toBe(3);
  });

  it('totals every record exactly once however the tree is shaped', () => {
    const records = [
      record(1, null, { cpu: 0.5, memory: 10 }),
      record(2, 1, { cpu: 1.5, memory: 20 }),
      record(3, 2, { cpu: 2.5, memory: 30 }),
      record(4, 99, { cpu: 3.5, memory: 40 }), // orphan
      record(5, 6, { cpu: 4.5, memory: 50 }), // cycle with 6
      record(6, 5, { cpu: 5.5, memory: 60 }),
      record(7, 7, { cpu: 6.5, memory: 70 }), // self-parenting
    ];
    const expectedCpu = records.reduce((sum, r) => sum + r.cpu, 0);
    const expectedMemory = records.reduce((sum, r) => sum + r.memory, 0);

    const tree = buildTree(records);

    expect(tree.total.cpu).toBeCloseTo(expectedCpu);
    expect(tree.total.memory).toBe(expectedMemory);
    expect(tree.total.processes).toBe(records.length);
  });
});

describe('flattenTree', () => {
  const tree = buildTree([
    record(1, null, { name: 'chrome', cpu: 1 }),
    record(2, 1, { name: 'renderer', cpu: 5 }),
    record(3, 1, { name: 'gpu-process', cpu: 3 }),
    record(10, null, { name: 'code', cpu: 8 }),
  ]);

  it('shows only roots when nothing is expanded', () => {
    const rows = flattenTree(tree, { expanded: new Set() });

    // Ordered by the default CPU sort, which reads the *roll-up*: chrome's own
    // 1% plus its children's 8% outranks code's 8%. Sorting on each row's own
    // figure would rank a busy parent below one of its own children.
    expect(rows.map((row) => row.node.record.name)).toEqual(['chrome', 'code']);
  });

  it('shows children of an expanded node', () => {
    const chrome = tree.roots.find((root) => root.record.name === 'chrome');
    const rows = flattenTree(tree, { expanded: new Set([chrome?.key ?? '']) });

    expect(rows.map((row) => row.node.record.name)).toEqual([
      'chrome',
      'renderer',
      'gpu-process',
      'code',
    ]);
  });

  it('indents children below their parent', () => {
    const chrome = tree.roots.find((root) => root.record.name === 'chrome');
    const rows = flattenTree(tree, { expanded: new Set([chrome?.key ?? '']) });

    expect(rows.find((row) => row.node.record.name === 'renderer')?.depth).toBe(1);
    expect(rows.find((row) => row.node.record.name === 'chrome')?.depth).toBe(0);
  });

  it('auto-expands to reveal a match, even when the parent is collapsed', () => {
    // A filter that reports a match and then hides it inside a collapsed parent
    // is worse than no filter at all.
    const rows = flattenTree(tree, { expanded: new Set(), search: 'renderer' });

    expect(rows.map((row) => row.node.record.name)).toEqual(['chrome', 'renderer']);
  });

  it('keeps a parent on the path to a match', () => {
    const rows = flattenTree(tree, { expanded: new Set(), search: 'gpu' });

    expect(rows.some((row) => row.node.record.name === 'chrome')).toBe(true);
  });

  it('returns nothing when a search matches nothing', () => {
    expect(flattenTree(tree, { expanded: new Set(), search: 'nothing-here' })).toEqual([]);
  });

  it('matches on PID and on user as well as name', () => {
    expect(flattenTree(tree, { expanded: new Set(), search: '10' })).toHaveLength(1);
    expect(flattenTree(tree, { expanded: new Set(), search: 'tester' }).length).toBeGreaterThan(0);
  });

  it('sorts roots by the chosen column and direction', () => {
    const ascending = flattenTree(tree, {
      expanded: new Set(),
      sort: { column: 'name', direction: 'asc' },
    });

    expect(ascending.map((row) => row.node.record.name)).toEqual(['chrome', 'code']);
  });

  it('lists every process at depth zero in flat mode', () => {
    const rows = flattenTree(tree, { expanded: new Set(), flat: true });

    expect(rows).toHaveLength(4);
    expect(rows.every((row) => row.depth === 0)).toBe(true);
  });

  it('marks a childless node as not expandable', () => {
    const rows = flattenTree(tree, { expanded: new Set() });

    expect(rows.find((row) => row.node.record.name === 'code')?.expandable).toBe(false);
    expect(rows.find((row) => row.node.record.name === 'chrome')?.expandable).toBe(true);
  });
});

describe('applyDiff', () => {
  const initial = new Map([[keyOf(record(1, null)), record(1, null)]]);

  it('adds a new process', () => {
    const next = applyDiff(initial, { added: [record(2, null)], removed: [], changed: [] });

    expect(next.size).toBe(2);
  });

  it('removes a departed process by its full identity', () => {
    const next = applyDiff(initial, {
      added: [],
      removed: [{ pid: 1, startedAt: 1000 }],
      changed: [],
    });

    expect(next.size).toBe(0);
  });

  it('does not remove a process whose PID matches but whose start time does not', () => {
    // Guards against a stale removal wiping out the process that recycled the PID.
    const next = applyDiff(initial, {
      added: [],
      removed: [{ pid: 1, startedAt: 999 }],
      changed: [],
    });

    expect(next.size).toBe(1);
  });

  it('updates a changed process in place', () => {
    const next = applyDiff(initial, {
      added: [],
      removed: [],
      changed: [record(1, null, { cpu: 55 })],
    });

    expect(next.get(keyOf(record(1, null)))?.cpu).toBe(55);
  });

  it('does not modify the map it was given', () => {
    applyDiff(initial, { added: [record(9, null)], removed: [], changed: [] });

    expect(initial.size).toBe(1);
  });
});
