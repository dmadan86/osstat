/**
 * The inference runtime section of Settings.
 *
 * osstat cannot run a model without a llama.cpp server, which is too large to
 * ship in a ~20 MB installer. This is where one is fetched, and where what was
 * fetched can be seen and deleted — osstat is a disk cleaner, so leaving
 * hundreds of megabytes around that it does not admit to would be the exact
 * behaviour it exists to complain about.
 *
 * Two decisions are visible in the markup and worth stating:
 *
 * **Both options are shown with their sizes before anything starts.** Vulkan is
 * ~34 MB and CUDA ~538 MB, so presenting the larger as a consequence of owning
 * an NVIDIA card would be making the user's decision for them.
 *
 * **A verification failure gets no retry control.** The backend decides this,
 * not the markup: `RuntimeFailure.retryable` is false for a checksum mismatch,
 * because retrying a hash that did not match either wastes a 600 MB download or
 * invites someone to keep trying until a tampered file slips through.
 *
 * Its own file rather than another section inside `Settings.tsx`: this one owns
 * an event subscription and four states, where every other section is a control
 * bound to a preference.
 */

import { useEffect, useState } from 'react';

import type { RuntimeFailure } from '../bindings/RuntimeFailure';
import type { RuntimeOption } from '../bindings/RuntimeOption';
import type { RuntimeProgress } from '../bindings/RuntimeProgress';
import type { RuntimeStatus } from '../bindings/RuntimeStatus';
import { formatBytes } from '../lib/format';
import {
  acquireRuntime,
  deleteRuntime,
  fetchRuntimeStatus,
  onRuntimeFailed,
  onRuntimeProgress,
  onRuntimeReady,
} from '../lib/ipc';

/** What the section is doing. */
type LoadState =
  | { status: 'loading' }
  | { status: 'probing' }
  | { status: 'ready'; runtime: RuntimeStatus }
  | { status: 'error'; message: string };

/** Renders an unknown thrown value as a message. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Fetches the status and turns it into a state.
 *
 * Module scope rather than a `useCallback`, and it returns the next state
 * instead of setting it: an effect calling a local function that sets state is
 * the cascading-render pattern `react-hooks/set-state-in-effect` catches.
 * `Llm.tsx` and `Ports.tsx` use the same shape.
 */
async function loadRuntimeStatus(): Promise<LoadState> {
  const runtime = await fetchRuntimeStatus();

  // `null` is the GPU probe still running, not a failure — recommending a CPU
  // build on a machine that has a GPU is the wrong answer, confidently given.
  //
  // `undefined` is checked as well as `null`. A `Some(T)` that serialises to
  // `null` and an absent value are the same thing to this component, and
  // treating only one of them as "not ready yet" means the other renders as a
  // status object with no fields — which fails several frames later, nowhere
  // near the cause.
  return runtime === null || runtime === undefined
    ? { status: 'probing' }
    : { status: 'ready', runtime };
}

/** What each progress phase is called on screen. */
const PHASES: Record<string, string> = {
  downloading: 'Downloading',
  cudaRuntime: 'Downloading the CUDA runtime',
  verifying: 'Verifying and unpacking',
};

/** Renders the inference runtime section. */
export function InferenceRuntime(): React.JSX.Element {
  const [state, setState] = useState<LoadState>({ status: 'loading' });
  const [progress, setProgress] = useState<RuntimeProgress | null>(null);
  const [failure, setFailure] = useState<RuntimeFailure | null>(null);

  const [reloadToken, setReloadToken] = useState(0);

  useEffect(() => {
    let cancelled = false;

    loadRuntimeStatus().then(
      (next) => {
        if (!cancelled) setState(next);
      },
      (error: unknown) => {
        if (!cancelled) setState({ status: 'error', message: messageOf(error) });
      }
    );

    return () => {
      cancelled = true;
    };
  }, [reloadToken]);

  useEffect(() => {
    const subscriptions = [
      onRuntimeProgress((next) => {
        setProgress(next);
      }),
      onRuntimeReady(() => {
        setProgress(null);
        setFailure(null);
        setReloadToken((token) => token + 1);
      }),
      onRuntimeFailed((next) => {
        setProgress(null);
        setFailure(next);
      }),
    ];

    return () => {
      for (const subscription of subscriptions) {
        subscription.then(
          (off) => {
            off();
          },
          () => {
            // Nothing to unsubscribe from if the subscription itself failed.
          }
        );
      }
    };
  }, []);

  function install(acceptCuda: boolean): void {
    setFailure(null);
    setProgress({ phase: 'downloading', downloadedBytes: 0, totalBytes: 0 });

    acquireRuntime(acceptCuda).catch((error: unknown) => {
      setProgress(null);
      setFailure({
        message: messageOf(error),
        retryable: true,
        verificationFailure: false,
      });
    });
  }

  function remove(tag: string, artifactId: string): void {
    deleteRuntime(tag, artifactId).then(
      () => {
        setReloadToken((token) => token + 1);
      },
      (error: unknown) => {
        setFailure({
          message: messageOf(error),
          retryable: false,
          verificationFailure: false,
        });
      }
    );
  }

  const busy = progress !== null;

  return (
    <section aria-label="Inference runtime" className="border-t border-edge p-3">
      <p className="text-sm">Inference runtime</p>
      <p className="mt-0.5 text-xs text-text-muted">
        The llama.cpp server osstat runs models with. It is downloaded when you ask for it, never
        before, and osstat refuses to run a build whose checksum does not match the one pinned in
        its source.
      </p>

      {state.status === 'loading' && (
        <p className="mt-3 text-xs text-text-muted">Checking what is installed…</p>
      )}

      {state.status === 'probing' && (
        <p className="mt-3 text-xs text-text-muted">
          Waiting for the GPU probe. Recommending a build before it answers would mean offering a
          CPU runtime to a machine that has a graphics card.
        </p>
      )}

      {state.status === 'error' && (
        <p role="alert" className="mt-3 text-xs text-red-400">
          Could not read the runtime status: {state.message}
        </p>
      )}

      {state.status === 'ready' && (
        <>
          {state.runtime.installed.length > 0 && (
            <ul className="mt-3 flex flex-col gap-1.5">
              {state.runtime.installed.map((runtime) => (
                <li
                  key={`${runtime.tag}/${runtime.artifactId}`}
                  className="flex items-center justify-between gap-3 rounded-md border border-edge px-2.5 py-1.5"
                >
                  <span className="font-mono text-xs">
                    <span data-selectable>{runtime.artifactId}</span>
                    <span className="ml-2 text-text-muted">{runtime.tag}</span>
                    <span className="ml-2 text-text-muted">{formatBytes(runtime.sizeBytes)}</span>
                  </span>
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => {
                      remove(runtime.tag, runtime.artifactId);
                    }}
                    className="rounded-md border border-edge px-2 py-0.5 text-xs text-text-muted hover:bg-white/[0.04] disabled:opacity-40"
                  >
                    Delete
                  </button>
                </li>
              ))}
            </ul>
          )}

          {state.runtime.recommended === null && (
            <p role="alert" className="mt-3 text-xs text-amber-400/80">
              llama.cpp publishes no build for this operating system and architecture, so osstat
              cannot run a model here.
            </p>
          )}

          {state.runtime.recommended !== null && !busy && (
            <div className="mt-3 flex flex-wrap gap-2">
              <InstallButton
                option={state.runtime.recommended}
                recommended
                onInstall={() => {
                  install(false);
                }}
              />
              {state.runtime.cudaAlternative !== null && (
                <InstallButton
                  option={state.runtime.cudaAlternative}
                  recommended={false}
                  onInstall={() => {
                    install(true);
                  }}
                />
              )}
            </div>
          )}

          {state.runtime.cudaAlternative !== null && !busy && (
            <p className="mt-2 text-[11px] text-text-faint">
              CUDA is faster on NVIDIA hardware. It is not the default because it is a much larger
              download — the CUDA runtime ships as a second archive.
            </p>
          )}
        </>
      )}

      {progress !== null && (
        <p role="status" className="mt-3 font-mono text-xs text-text-muted">
          {PHASES[progress.phase] ?? 'Working'}
          {progress.totalBytes > 0 &&
            ` — ${formatBytes(progress.downloadedBytes)} of ${formatBytes(progress.totalBytes)}`}
          …
        </p>
      )}

      {failure !== null && (
        <div role="alert" className="mt-3 text-xs">
          <p className={failure.verificationFailure ? 'text-red-400' : 'text-amber-400/90'}>
            {failure.verificationFailure
              ? 'Verification failed. osstat did not unpack or run anything, and removed what it downloaded.'
              : 'The download did not finish.'}
          </p>
          <p className="mt-1 font-mono text-[11px] text-text-muted">{failure.message}</p>
          {failure.retryable && (
            <button
              type="button"
              onClick={() => {
                install(false);
              }}
              className="mt-2 rounded-md border border-edge px-2 py-0.5 text-xs text-text hover:bg-white/[0.04]"
            >
              Try again
            </button>
          )}
        </div>
      )}
    </section>
  );
}

/** One installable option, with what it would cost. */
function InstallButton({
  option,
  recommended,
  onInstall,
}: {
  option: RuntimeOption;
  recommended: boolean;
  onInstall: () => void;
}): React.JSX.Element {
  return (
    <button
      type="button"
      onClick={onInstall}
      className={`rounded-md border px-2.5 py-1 text-xs transition-colors ${
        recommended
          ? 'border-accent bg-accent/10 text-accent'
          : 'border-edge text-text-muted hover:bg-white/[0.04]'
      }`}
    >
      Install {option.backend}
      <span className="ml-2 font-mono text-[11px] opacity-70">
        {formatBytes(option.downloadBytes)}
      </span>
    </button>
  );
}
