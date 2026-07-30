import { describe, expect, it } from 'vitest';

import { GRACE_MS, matchCritical, WAIT_CEILING_MS } from './termination';
import type { CriticalProcess } from '../bindings/CriticalProcess';

const LIST: CriticalProcess[] = [
  {
    platform: 'windows',
    name: 'explorer.exe',
    paths: ['\\Windows\\explorer.exe'],
    breaks: 'the taskbar',
  },
];

describe('matchCritical', () => {
  it('matches the right name at the right path', () => {
    expect(matchCritical(LIST, 'explorer.exe', 'C:\\Windows\\explorer.exe')).not.toBeNull();
  });

  it('does not match the right name at the wrong path', () => {
    // A downloaded binary must not inherit the real shell's protection, which
    // would make an unknown process harder to end than a system one.
    expect(matchCritical(LIST, 'explorer.exe', 'C:\\Users\\me\\explorer.exe')).toBeNull();
  });

  it('does not match a different name at a protected path', () => {
    expect(matchCritical(LIST, 'notepad.exe', 'C:\\Windows\\notepad.exe')).toBeNull();
  });

  it('treats an unreadable path as not critical', () => {
    expect(matchCritical(LIST, 'explorer.exe', null)).toBeNull();
  });

  it('ignores case and separator style', () => {
    expect(matchCritical(LIST, 'EXPLORER.EXE', 'd:/WINDOWS/EXPLORER.EXE')).not.toBeNull();
  });
});

describe('the wait', () => {
  it('gives up well after the grace period, not at it', () => {
    // The tick is 2 s and slows in the background, so detection lands past
    // 5 s. A ceiling equal to the grace period would race the stream.
    expect(WAIT_CEILING_MS).toBeGreaterThan(GRACE_MS * 2);
  });
});
