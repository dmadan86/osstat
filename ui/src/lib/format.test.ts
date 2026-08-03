import { describe, expect, it } from 'vitest';

import {
  formatBytes,
  formatCount,
  formatDuration,
  formatFraction,
  formatFrequency,
  formatPercent,
  formatRate,
  formatTimeOfDay,
} from './format';

describe('formatBytes', () => {
  it('scales to a binary unit', () => {
    expect(formatBytes(0)).toBe('0 B');
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(1024)).toBe('1.00 KB');
    expect(formatBytes(1024 * 1024 * 1024)).toBe('1.00 GB');
  });

  it('keeps about three significant figures as the unit grows', () => {
    expect(formatBytes(1_500_000_000)).toBe('1.40 GB');
    expect(formatBytes(888_000_000)).toBe('847 MB');
  });

  it('reports a dash rather than NaN for nonsense input', () => {
    expect(formatBytes(Number.NaN)).toBe('—');
    expect(formatBytes(-1)).toBe('—');
  });
});

describe('formatRate', () => {
  it('appends a per-second suffix', () => {
    expect(formatRate(1024)).toBe('1.00 KB/s');
  });

  it('shows a dash for no throughput, rather than a distracting zero', () => {
    expect(formatRate(0)).toBe('—');
  });
});

describe('formatPercent', () => {
  it('renders at the tenth of a percent the backend diffs on', () => {
    // Any more precision would show digits that update less often than they change.
    expect(formatPercent(12.34)).toBe('12.3%');
    expect(formatPercent(0)).toBe('0.0%');
  });

  it('allows above 100 for a multi-core figure', () => {
    expect(formatPercent(340)).toBe('340.0%');
  });
});

describe('formatFraction', () => {
  it('renders a ratio as whole percent', () => {
    expect(formatFraction(0.66)).toBe('66%');
    expect(formatFraction(1)).toBe('100%');
  });
});

describe('formatCount', () => {
  it('groups thousands', () => {
    expect(formatCount(44)).toBe('44');
    expect(formatCount(1048)).toBe('1,048');
    expect(formatCount(4096)).toBe('4,096');
    expect(formatCount(131_072)).toBe('131,072');
  });

  it('reports a dash rather than NaN for nonsense input', () => {
    expect(formatCount(Number.NaN)).toBe('—');
  });
});

describe('formatDuration', () => {
  it('uses coarse units, largest first', () => {
    expect(formatDuration(2 * 86_400 + 4 * 3_600)).toBe('2 d 4 h');
    expect(formatDuration(3 * 3_600 + 25 * 60)).toBe('3 h 25 m');
    expect(formatDuration(19 * 60)).toBe('19 m');
    expect(formatDuration(44)).toBe('44 s');
  });

  it('reports a dash for nonsense input', () => {
    expect(formatDuration(-5)).toBe('—');
  });
});

describe('formatFrequency', () => {
  it('switches to gigahertz above a thousand megahertz', () => {
    expect(formatFrequency(4500)).toBe('4.50 GHz');
    expect(formatFrequency(800)).toBe('800 MHz');
  });

  it('reports a dash when the platform did not say', () => {
    // sysinfo returns zero rather than null for an unknown frequency.
    expect(formatFrequency(0)).toBe('—');
  });
});

describe('formatTimeOfDay', () => {
  it('renders the hour and minute of the instant, and no seconds', () => {
    // Built from the local zone rather than pinned to a literal: the test runs
    // wherever CI puts it, and `09:15` would be right only in UTC.
    const at = new Date(2026, 7, 3, 9, 15, 42);
    const expected = `${String(at.getHours()).padStart(2, '0')}:${String(at.getMinutes()).padStart(
      2,
      '0'
    )}`;

    const rendered = formatTimeOfDay(at.getTime());

    expect(rendered).toBe(expected);
    // The 42 seconds are not in it; a transcript stamps a turn, not a sample.
    expect(rendered).not.toContain('42');
  });

  it('pads a single-digit hour so a column of times lines up', () => {
    const at = new Date(2026, 7, 3, 4, 5, 0);

    expect(formatTimeOfDay(at.getTime())).toBe(
      `${String(at.getHours()).padStart(2, '0')}:${String(at.getMinutes()).padStart(2, '0')}`
    );
    expect(formatTimeOfDay(at.getTime())).toHaveLength(5);
  });

  it('reports a dash rather than an invalid date', () => {
    expect(formatTimeOfDay(Number.NaN)).toBe('—');
  });
});
