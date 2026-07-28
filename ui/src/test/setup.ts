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

// jsdom does not implement pointer capture at all -- calling either method
// throws "is not a function". `useDragReorder` captures the pointer on drag
// start so a release outside the viewport still reaches `document`; the
// capture itself has no observable effect in jsdom (which lays nothing out
// and delivers events to `document` regardless), so a no-op is enough.
if (typeof HTMLElement.prototype.setPointerCapture === 'undefined') {
  HTMLElement.prototype.setPointerCapture = (): void => {};
}
if (typeof HTMLElement.prototype.releasePointerCapture === 'undefined') {
  HTMLElement.prototype.releasePointerCapture = (): void => {};
}

afterEach(() => {
  cleanup();
});
