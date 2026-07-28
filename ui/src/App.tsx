import { useEffect, useState } from 'react';
import { fetchAppInfo } from './lib/ipc';
import type { AppInfo } from './bindings/AppInfo';

/**
 * The capability areas from the roadmap. Shown here so the M0 shell states
 * plainly what does and does not exist yet rather than implying a finished app.
 */
const CAPABILITIES = [
  { name: 'System info', milestone: 'M1', summary: 'OS, CPU, memory, disks, GPUs' },
  { name: 'Processes', milestone: 'M1', summary: 'Process tree with permission-aware kill' },
  { name: 'Ports', milestone: 'M2', summary: 'Which process holds which socket' },
  { name: 'Cleaner', milestone: 'M3', summary: 'Rule-driven, preview-first disk reclaim' },
  { name: 'LLM advisor', milestone: 'M4', summary: 'Which local models this machine can run' },
] as const;

type LoadState =
  { status: 'loading' } | { status: 'ready'; info: AppInfo } | { status: 'error'; message: string };

export function App() {
  const [state, setState] = useState<LoadState>({ status: 'loading' });

  useEffect(() => {
    let cancelled = false;

    fetchAppInfo()
      .then((info) => {
        if (!cancelled) {
          setState({ status: 'ready', info });
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          const message = error instanceof Error ? error.message : String(error);
          setState({ status: 'error', message });
        }
      });

    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <main className="mx-auto flex h-full max-w-3xl flex-col justify-center gap-8 px-8 py-12">
      <header className="flex items-baseline gap-3">
        <h1 className="text-3xl font-semibold tracking-tight">osstat</h1>
        <p className="text-sm text-neutral-400">
          System cleaner, inspector and LLM runnability advisor
        </p>
      </header>

      <section
        aria-label="Build information"
        className="rounded-xl border border-edge bg-surface-raised p-6"
      >
        {state.status === 'loading' && (
          <p role="status" className="text-sm text-neutral-400">
            Loading build information…
          </p>
        )}

        {state.status === 'error' && (
          <p role="alert" className="text-sm text-red-400">
            Could not reach the osstat backend: {state.message}
          </p>
        )}

        {state.status === 'ready' && (
          <dl className="grid grid-cols-[auto_1fr] gap-x-8 gap-y-2 text-sm">
            <dt className="text-neutral-400">Version</dt>
            <dd data-selectable className="font-mono">
              {state.info.build.version}
            </dd>

            <dt className="text-neutral-400">Platform</dt>
            <dd data-selectable>{state.info.platformName}</dd>

            <dt className="text-neutral-400">Build</dt>
            <dd data-selectable className="font-mono">
              {state.info.build.profile}
            </dd>
          </dl>
        )}
      </section>

      <section aria-label="Planned capabilities" className="flex flex-col gap-2">
        <h2 className="text-xs font-medium uppercase tracking-wider text-neutral-500">
          Not built yet
        </h2>
        <ul className="divide-y divide-edge overflow-hidden rounded-xl border border-edge">
          {CAPABILITIES.map((capability) => (
            <li
              key={capability.name}
              className="flex items-center justify-between gap-4 px-4 py-3 text-sm"
            >
              <span className="flex flex-col">
                <span>{capability.name}</span>
                <span className="text-xs text-neutral-500">{capability.summary}</span>
              </span>
              <span className="rounded-full border border-edge px-2 py-0.5 font-mono text-xs text-neutral-400">
                {capability.milestone}
              </span>
            </li>
          ))}
        </ul>
      </section>
    </main>
  );
}
