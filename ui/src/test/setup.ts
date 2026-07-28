import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import { afterEach } from 'vitest';

// jsdom lays nothing out and does not implement ResizeObserver at all. A test
// that needs real measurements stubs it itself; this fallback only keeps
// components that observe an element (to react to resizing) from crashing
// everywhere else, by never calling back.
if (typeof globalThis.ResizeObserver === 'undefined') {
  class NoopResizeObserver implements ResizeObserver {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  }

  globalThis.ResizeObserver = NoopResizeObserver;
}

afterEach(() => {
  cleanup();
});
