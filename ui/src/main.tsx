import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import './index.css';

/**
 * Renders a startup failure into the page.
 *
 * A webview that throws before React mounts paints nothing, and a blank white
 * window tells a user precisely nothing — not even whether the app is still
 * loading. It is also unreportable: there is no message to paste into an issue.
 * Writing the error where the UI would have been costs a few lines and turns
 * "it just doesn't open" into something actionable.
 */
function reportStartupFailure(error: unknown): void {
  const message =
    error instanceof Error ? `${error.message}\n\n${error.stack ?? ''}` : String(error);

  document.body.innerHTML = '';
  const pre = document.createElement('pre');
  pre.setAttribute('data-selectable', '');
  pre.style.cssText =
    'margin:0;padding:24px;font:12px/1.6 ui-monospace,monospace;color:#ff9c9c;' +
    'background:#0f141d;white-space:pre-wrap;word-break:break-word;height:100%;overflow:auto';
  pre.textContent = `osstat failed to start.\n\n${message}`;
  document.body.appendChild(pre);
}

window.addEventListener('error', (event) => {
  reportStartupFailure(event.error ?? event.message);
});

window.addEventListener('unhandledrejection', (event) => {
  reportStartupFailure(event.reason);
});

try {
  const container = document.getElementById('root');

  if (!container) {
    throw new Error('index.html is missing the #root mount point');
  }

  createRoot(container).render(
    <StrictMode>
      <App />
    </StrictMode>
  );
} catch (error) {
  reportStartupFailure(error);
}
