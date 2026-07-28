import { bench, describe } from 'vitest';

import type { ProcessRecord } from '../bindings/ProcessRecord';
import { buildTree, flattenTree } from './processTree';

/**
 * The front-end half of the M1 gate.
 *
 * ROADMAP.md commits to a 500-process tree refreshing in under 50 ms. That
 * budget is shared: the Rust side reads the process table and diffs it (see
 * `crates/osstat-platform/benches/process_refresh.rs`), and this side arranges
 * the result into the tree that gets rendered. Benchmarking only the half
 * written in Rust would measure a number nobody experiences.
 */

/** A process table shaped like a real one: a few roots, the rest beneath them. */
function table(count: number): ProcessRecord[] {
  return Array.from({ length: count }, (_, index) => ({
    pid: index + 1,
    parentPid: index < 8 ? null : (index % 8) + 1,
    name: `process-${index}.exe`,
    exe: `C:\\Program Files\\app\\process-${index}.exe`,
    user: 'tester',
    cpu: (index % 100) / 10,
    memory: index * 1_048_576,
    diskReadRate: index * 64,
    diskWriteRate: index * 32,
    startedAt: 1_700_000_000 + index,
    status: 'running' as const,
  }));
}

describe('process tree', () => {
  const records = table(500);
  const large = table(2000);
  const tree = buildTree(records);
  const expanded = new Set(tree.roots.map((root) => root.key));

  bench('buildTree, 500 processes', () => {
    buildTree(records);
  });

  bench('buildTree, 2000 processes', () => {
    buildTree(large);
  });

  bench('buildTree + flatten, 500 processes', () => {
    flattenTree(buildTree(records), { expanded });
  });

  bench('flatten with a search, 500 processes', () => {
    flattenTree(tree, { expanded: new Set(), search: 'process-4' });
  });
});
