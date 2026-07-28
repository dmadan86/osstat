import { describe, expect, it } from 'vitest';

import type { PortRecord } from '../bindings/PortRecord';
import { filterPorts, matchesFilter } from './ports';

/** Builds a row with sane defaults, overridden per test. */
function row(overrides: Partial<PortRecord> = {}): PortRecord {
  return {
    protocol: 'tcp',
    localAddress: '127.0.0.1',
    localPort: 8080,
    remoteAddress: null,
    remotePort: null,
    state: 'listen',
    pid: 42,
    processName: 'web-server',
    processPath: '/usr/bin/web-server',
    ...overrides,
  };
}

describe('matchesFilter', () => {
  it('matches everything when both halves are blank', () => {
    expect(matchesFilter(row(), {})).toBe(true);
    expect(matchesFilter(row(), { port: '  ', process: '' })).toBe(true);
  });

  it('matches a local port substring', () => {
    expect(matchesFilter(row({ localPort: 8080 }), { port: '808' })).toBe(true);
    expect(matchesFilter(row({ localPort: 8080 }), { port: '9090' })).toBe(false);
  });

  it('matches a remote port when there is one', () => {
    const withPeer = row({ remoteAddress: '10.0.0.5', remotePort: 443 });
    expect(matchesFilter(withPeer, { port: '443' })).toBe(true);
  });

  it('does not match a remote port that is absent', () => {
    const noPeer = row({ remoteAddress: null, remotePort: null });
    expect(matchesFilter(noPeer, { port: '443' })).toBe(false);
  });

  it('matches a process name case-insensitively', () => {
    expect(matchesFilter(row({ processName: 'nginx' }), { process: 'NGI' })).toBe(true);
    expect(matchesFilter(row({ processName: 'nginx' }), { process: 'apache' })).toBe(false);
  });

  it('matches a PID as text', () => {
    expect(matchesFilter(row({ pid: 4242 }), { process: '424' })).toBe(true);
  });

  it('treats a row with no attributed process as unmatched by a process filter', () => {
    const orphan = row({ pid: null, processName: null });
    expect(matchesFilter(orphan, { process: 'anything' })).toBe(false);
  });

  it('requires both halves to match at once', () => {
    const target = row({ localPort: 8080, processName: 'nginx' });
    expect(matchesFilter(target, { port: '8080', process: 'nginx' })).toBe(true);
    expect(matchesFilter(target, { port: '8080', process: 'apache' })).toBe(false);
    expect(matchesFilter(target, { port: '9999', process: 'nginx' })).toBe(false);
  });
});

describe('filterPorts', () => {
  it('sorts listening sockets ahead of everything else', () => {
    const rows = [
      row({ localPort: 1000, state: 'established' }),
      row({ localPort: 2000, state: 'listen' }),
    ];

    const sorted = filterPorts(rows, {});

    expect(sorted.map((r) => r.localPort)).toEqual([2000, 1000]);
  });

  it('orders same-status rows by local port ascending', () => {
    const rows = [row({ localPort: 3000 }), row({ localPort: 1000 }), row({ localPort: 2000 })];

    const sorted = filterPorts(rows, {});

    expect(sorted.map((r) => r.localPort)).toEqual([1000, 2000, 3000]);
  });

  it('drops rows that fail the filter without mutating the input', () => {
    const rows = [row({ localPort: 80 }), row({ localPort: 443 })];

    const filtered = filterPorts(rows, { port: '443' });

    expect(filtered).toHaveLength(1);
    expect(filtered[0]?.localPort).toBe(443);
    expect(rows).toHaveLength(2);
  });
});
