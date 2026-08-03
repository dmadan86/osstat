/**
 * The LLM runnability advisor.
 *
 * Three parts, in the order ROADMAP.md M4 names them: a hardware card saying
 * what this machine has, a fit matrix of every model at every quantization,
 * and a drawer showing the arithmetic behind whichever cell you pick.
 *
 * Every cell also carries the one thing a verdict cannot do on its own: a way
 * to actually get the file. Six states, and the difference between them is the
 * feature — *not pinned* says so rather than offering a control that would
 * fail, *downloadable* names the publisher because these are community
 * re-quantizations rather than the vendors' own uploads, *downloading* counts
 * against the pinned size and offers Pause and Cancel, *paused* keeps that
 * figure and offers Resume, *failed* offers Retry once the automatic attempts
 * are used up, and *downloaded* offers Run, which hands the record's absolute
 * path to the same `chat_open_model` the file picker uses.
 *
 * Pause and Cancel are both offered because they do different things to the
 * disk: Pause keeps the partial file and Cancel deletes it. A single Stop would
 * make that choice on the user's behalf, silently, with several gigabytes.
 *
 * **A download is offered even where the verdict says the model will not fit.**
 * The calculator is an estimate — ADR-008 says so in its first paragraph — and
 * refusing on it would make osstat wrong in a way the user cannot override. The
 * verdict sits beside the control instead, so the choice is informed rather
 * than removed.
 *
 * The drawer is the point of the page rather than a detail of it. ADR-008 is
 * explicit that presenting a heuristic as a measurement is the worst thing
 * this feature could do, so every verdict is one click from the terms that
 * produced it — and the caveats (no VRAM figure, a context past a model's own
 * maximum) are shown next to the verdict, not buried in a footnote.
 *
 * Like Ports this fetches rather than following a tick: the answer only
 * changes when the hardware probe finishes or the user moves the context
 * control, so it listens for `gpus:ready` and otherwise re-fetches on demand.
 */

import { useEffect, useMemo, useState } from 'react';

import type { FitResult } from '../bindings/FitResult';
import type { LlmAdvice } from '../bindings/LlmAdvice';
import type { ModelCatalogueEntry } from '../bindings/ModelCatalogueEntry';
import type { ModelEntry } from '../bindings/ModelEntry';
import type { ModelFailure } from '../bindings/ModelFailure';
import type { ModelKey } from '../bindings/ModelKey';
import type { ModelRegistry } from '../bindings/ModelRegistry';
import type { ModelSession } from '../bindings/ModelSession';
import type { QuantLevel } from '../bindings/QuantLevel';
import {
  budgetCaveat,
  cellKey,
  CONTEXT_LENGTHS,
  countByVerdict,
  exceedsNativeContext,
  formatTokens,
  indexCatalogue,
  indexResults,
  SPEED_TIERS,
  VERDICTS,
} from '../lib/advisor';
import { formatBytes, formatDuration, formatRate } from '../lib/format';
import {
  cancelModelDownload,
  chatOpenModel,
  downloadModel,
  fetchLlmAdvice,
  fetchModelCatalogue,
  fetchModelRegistry,
  onGpusReady,
  onModelDone,
  onModelFailed,
  onModelProgress,
  pauseModelDownload,
} from '../lib/ipc';
import { Meter } from '../components/Meter';

/** What a load can be doing. */
type LoadState =
  | { status: 'loading' }
  | { status: 'probing' }
  | { status: 'ready'; registry: ModelRegistry; advice: LlmAdvice }
  | { status: 'error'; message: string };

/**
 * The download under way, as far as it has got.
 *
 * Held here rather than read from the catalogue's `state` because the catalogue
 * is fetched, not streamed: between clicking Download and the first
 * `model:progress` there would otherwise be a cell still offering Download for
 * a file already being fetched.
 *
 * It survives a **pause** and not a cancel, which is the whole difference
 * between the two: after a pause the figure still describes bytes on disk, and
 * after a cancel those bytes have been deleted, so a bar still reading 2.29 GB
 * would promise a resume that would in fact start from zero.
 */
interface Transfer {
  /** Which cell is downloading. */
  key: ModelKey;
  /** Bytes that have landed, including anything a resume already had. */
  downloadedBytes: number;
  /** Bytes expected, from the pin rather than from any response. */
  totalBytes: number;
  /** Bytes per second over the backend's window, or `null` if it cannot say. */
  bytesPerSecond: number | null;
  /** Seconds left at that rate, or `null` when there is no honest estimate. */
  secondsRemaining: number | null;
  /** Whether the user paused it, so the controls offer Resume rather than Pause. */
  paused: boolean;
}

/** Which of the six things a cell can be doing. */
type Phase = 'unpinned' | 'downloadable' | 'downloading' | 'paused' | 'failed' | 'downloaded';

/** Which cell the drawer is open on. */
interface Selection {
  /** The model that row describes. */
  model: ModelEntry;
  /** The quantization that column describes. */
  quant: QuantLevel;
  /** The verdict and its arithmetic. */
  result: FitResult;
}

/** What the advisor needs from the shell. */
export interface LlmProps {
  /**
   * Called with the session the Run control opened, so the shell can show it.
   *
   * The advisor opens the model itself rather than handing a path onwards:
   * `chat_open_model` is the one command that starts a session, and routing the
   * path through a second caller would be a second way in.
   */
  onModelOpened?: (session: ModelSession) => void;
}

/** Renders an unknown thrown value as a message. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Fetches both halves of the page's data and turns them into a state.
 *
 * Module scope rather than a `useCallback` inside the component, and it
 * returns the next state instead of setting it: an effect that calls a local
 * function which sets state is the cascading-render pattern
 * `react-hooks/set-state-in-effect` exists to catch, and the same shape
 * `Ports` uses — resolve here, set state in the promise callback — keeps the
 * two pages consistent as well as quiet.
 */
async function loadAdvice(tokens: number): Promise<LoadState> {
  const [registry, advice] = await Promise.all([fetchModelRegistry(), fetchLlmAdvice(tokens)]);

  // `null` is the probe still running, not a failure — the same convention
  // `gpu_devices` uses. The `gpus:ready` subscription re-runs this.
  return advice === null ? { status: 'probing' } : { status: 'ready', registry, advice };
}

/**
 * Renders the LLM runnability advisor.
 *
 * @param props Where an opened session should be sent.
 */
export function Llm({ onModelOpened }: LlmProps = {}): React.JSX.Element {
  const [contextLength, setContextLength] = useState(4096);
  const [state, setState] = useState<LoadState>({ status: 'loading' });
  const [selected, setSelected] = useState<Selection | null>(null);
  const [catalogue, setCatalogue] = useState<ModelCatalogueEntry[]>([]);
  const [catalogueToken, setCatalogueToken] = useState(0);
  const [transfer, setTransfer] = useState<Transfer | null>(null);
  const [failure, setFailure] = useState<ModelFailure | null>(null);
  const [runProblem, setRunProblem] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    loadAdvice(contextLength).then(
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
  }, [contextLength]);

  // The probe answers once, well after mount on a machine with a slow driver.
  useEffect(() => {
    let cancelled = false;
    const unlisten = onGpusReady(() => {
      loadAdvice(contextLength).then(
        (next) => {
          if (!cancelled) setState(next);
        },
        (error: unknown) => {
          if (!cancelled) setState({ status: 'error', message: messageOf(error) });
        }
      );
    });

    return () => {
      cancelled = true;
      unlisten.then(
        (off) => {
          off();
        },
        () => {
          // Nothing to unsubscribe from if the subscription itself failed.
        }
      );
    };
  }, [contextLength]);

  // Independent of the context length: what is on disk does not change when the
  // KV cache is re-priced, so this re-runs only when a download or move ends.
  useEffect(() => {
    let cancelled = false;

    fetchModelCatalogue().then(
      (entries) => {
        if (!cancelled) setCatalogue(entries);
      },
      () => {
        // A catalogue that cannot be read leaves every cell saying "not
        // pinned", which is the honest reading of "osstat does not know of a
        // file here" and is not worth displacing the fit matrix over.
      }
    );

    return () => {
      cancelled = true;
    };
  }, [catalogueToken]);

  useEffect(() => {
    const subscriptions = [
      onModelProgress((next) => {
        // A `null` key is the whole library moving, which is Settings' business.
        if (next.key !== null) {
          setTransfer({
            key: next.key,
            downloadedBytes: next.downloadedBytes,
            totalBytes: next.totalBytes,
            bytesPerSecond: next.bytesPerSecond,
            secondsRemaining: next.secondsRemaining,
            paused: false,
          });
        }
      }),
      onModelDone((next) => {
        if (next.key === null) return;
        setTransfer(null);
        setFailure(null);
        setCatalogueToken((token) => token + 1);
      }),
      onModelFailed((next) => {
        if (next.key === null) return;

        // Three outcomes, and the payload distinguishes them so this does not
        // have to guess from the message.
        if (next.stopped === 'pause') {
          // The partial file is kept, so the figure the bar reached is still
          // true. Zeroing it here would say the gigabytes already fetched had
          // been thrown away — and no alert, because a pause the user asked
          // for is not something to warn them about.
          setTransfer((held) => (held === null ? null : { ...held, paused: true }));
          return;
        }

        setTransfer(null);

        // A cancel deleted the partial file and was deliberate. The cell goes
        // back to offering Download, and nothing is reported as wrong.
        setFailure(next.stopped === 'cancel' ? null : next);
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

  const entries = useMemo(() => indexCatalogue(catalogue), [catalogue]);

  /**
   * Starts or continues fetching one pinned file.
   *
   * `from` is what the bar should read until the first `model:progress`
   * arrives: zero for a fresh download, and whatever a paused one had reached
   * for a resume. Restarting the figure at zero on a resume would be the one
   * thing resuming exists to avoid.
   */
  function download(key: ModelKey, totalBytes: number, from = 0): void {
    setFailure(null);
    setRunProblem(null);
    setTransfer({
      key,
      downloadedBytes: from,
      totalBytes,
      // No rate yet: the window has one sample, and a figure carried over from
      // before the pause would describe a transfer that was not running.
      bytesPerSecond: null,
      secondsRemaining: null,
      paused: false,
    });

    downloadModel(key.modelId, key.quantId).catch((error: unknown) => {
      // A refusal here is a refusal before any request: no pinned file, another
      // download already running, or not enough room. It never reaches
      // `model:failed`, so it is turned into the same shape by hand.
      setTransfer(null);
      setFailure({
        key,
        message: messageOf(error),
        retryable: false,
        verificationFailure: false,
        stopped: null,
      });
    });
  }

  /** Pauses the download running, keeping the partial file to resume from. */
  function pause(): void {
    pauseModelDownload().catch(() => {
      // A download that has already finished has nothing left to stop.
    });
  }

  /**
   * Cancels the download running, deleting the partial file.
   *
   * Also clears whatever the cell was showing, because there may be nothing
   * left in Rust to stop — a cancel pressed on a *failed* cell is the user
   * dismissing it, and no `model:failed` will arrive to do this for us.
   */
  function cancel(): void {
    setTransfer(null);
    setFailure(null);

    cancelModelDownload().catch(() => {
      // A download that has already finished has nothing left to stop.
    });
  }

  /**
   * Opens a downloaded model and hands the session to the shell.
   *
   * The record's own absolute path, given to the same command the file picker
   * uses. Nothing here reconstructs a path from the folder and the file name.
   */
  function run(entry: ModelCatalogueEntry): void {
    if (entry.path === null) return;

    setRunProblem(null);
    chatOpenModel(entry.path).then(
      (session) => {
        onModelOpened?.(session);
      },
      (error: unknown) => {
        setRunProblem(messageOf(error));
      }
    );
  }

  return (
    <div className="flex h-full flex-col gap-3">
      <ContextControl
        value={contextLength}
        onChange={(tokens) => {
          setSelected(null);
          setContextLength(tokens);
        }}
      />

      {state.status === 'error' && (
        <p
          role="alert"
          className="rounded-xl border border-edge p-6 text-center text-sm text-red-400"
        >
          Could not weigh the model registry against this machine: {state.message}
        </p>
      )}

      {state.status === 'loading' && (
        <p
          role="status"
          className="rounded-xl border border-edge p-6 text-center text-sm text-neutral-500"
        >
          Reading the model registry…
        </p>
      )}

      {state.status === 'probing' && (
        <p
          role="status"
          className="rounded-xl border border-edge p-6 text-center text-sm text-neutral-500"
        >
          Looking for a GPU… the fit matrix waits for the probe rather than reporting “no GPU” on a
          machine that has one.
        </p>
      )}

      {failure !== null && <AcquisitionFailure failure={failure} />}

      {runProblem !== null && (
        <p role="alert" className="rounded-xl border border-edge p-3 text-xs text-red-400">
          The model could not be opened: {runProblem}
        </p>
      )}

      {state.status === 'ready' && (
        <>
          <HardwareCard advice={state.advice} />
          <Matrix
            registry={state.registry}
            advice={state.advice}
            catalogue={entries}
            transfer={transfer}
            failure={failure}
            selected={selected}
            onSelect={setSelected}
            onDownload={download}
            onPause={pause}
            onCancel={cancel}
            onRun={run}
          />
        </>
      )}

      {selected !== null && state.status === 'ready' && (
        <Drawer
          selection={selected}
          advice={state.advice}
          onClose={() => {
            setSelected(null);
          }}
        />
      )}
    </div>
  );
}

/** The context-length control the whole matrix is priced at. */
function ContextControl({
  value,
  onChange,
}: {
  value: number;
  onChange: (tokens: number) => void;
}): React.JSX.Element {
  return (
    <div className="flex flex-wrap items-center gap-2">
      <span className="text-xs text-neutral-500">Context length</span>
      <div role="group" aria-label="Context length" className="flex flex-wrap gap-1">
        {CONTEXT_LENGTHS.map((tokens) => (
          <button
            key={tokens}
            type="button"
            aria-pressed={tokens === value}
            onClick={() => {
              onChange(tokens);
            }}
            className={`rounded-md border px-2.5 py-1 font-mono text-xs transition-colors ${
              tokens === value
                ? 'border-accent bg-accent/10 text-accent'
                : 'border-edge text-neutral-400 hover:bg-white/[0.04]'
            }`}
          >
            {formatTokens(tokens)}
          </button>
        ))}
      </div>
      <p className="text-[11px] text-neutral-600">
        Longer context means a larger KV cache, which is memory the weights no longer get.
      </p>
    </div>
  );
}

/** What this machine has to offer, and what that does to the verdicts. */
function HardwareCard({ advice }: { advice: LlmAdvice }): React.JSX.Element {
  const caveat = budgetCaveat(advice.gpu);
  const counts = countByVerdict(advice.results);

  return (
    <section
      aria-label="This machine"
      className="rounded-xl border border-edge bg-surface-raised p-3"
    >
      <div className="flex flex-wrap gap-x-8 gap-y-2 text-sm">
        <Figure
          label="VRAM"
          value={
            advice.gpu.present
              ? advice.gpu.vramBytes === null
                ? 'not reported'
                : formatBytes(advice.gpu.vramBytes)
              : 'no GPU found'
          }
        />
        <Figure label="System memory" value={formatBytes(advice.systemMemoryBytes)} />
        <Figure label="Context" value={`${formatTokens(advice.contextLength)} tokens`} />
        <Figure
          label="Runs on the GPU"
          value={`${String(counts.fitsOnGpu)} of ${String(advice.results.length)}`}
        />
      </div>

      {caveat !== null && (
        <p className="mt-2 border-t border-edge pt-2 text-[11px] text-amber-400/80">{caveat}</p>
      )}
    </section>
  );
}

/** One labelled figure in the hardware card. */
function Figure({ label, value }: { label: string; value: string }): React.JSX.Element {
  return (
    <div className="flex flex-col">
      <span className="text-[10px] uppercase tracking-wider text-neutral-500">{label}</span>
      <span data-selectable className="font-mono">
        {value}
      </span>
    </div>
  );
}

/** Every model at every quantization, as one table. */
function Matrix({
  registry,
  advice,
  catalogue,
  transfer,
  failure,
  selected,
  onSelect,
  onDownload,
  onPause,
  onCancel,
  onRun,
}: {
  registry: ModelRegistry;
  advice: LlmAdvice;
  catalogue: Map<string, ModelCatalogueEntry>;
  transfer: Transfer | null;
  failure: ModelFailure | null;
  selected: Selection | null;
  onSelect: (selection: Selection) => void;
  onDownload: (key: ModelKey, totalBytes: number, from?: number) => void;
  onPause: () => void;
  onCancel: () => void;
  onRun: (entry: ModelCatalogueEntry) => void;
}): React.JSX.Element {
  const index = useMemo(() => indexResults(advice.results), [advice.results]);

  return (
    <div className="min-h-0 flex-1 overflow-auto rounded-xl border border-edge bg-surface-raised">
      <table className="w-full border-collapse text-xs">
        <caption className="sr-only">
          Which models this machine can run, at {formatTokens(advice.contextLength)} tokens of
          context
        </caption>
        <thead>
          <tr className="text-[10px] uppercase tracking-wider text-neutral-500">
            <th scope="col" className="sticky top-0 bg-surface-raised px-3 py-2 text-left">
              Model
            </th>
            {registry.quantLevels.map((quant) => (
              <th
                key={quant.id}
                scope="col"
                title={quant.description}
                className="sticky top-0 bg-surface-raised px-3 py-2 text-left font-mono"
              >
                {quant.label}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {registry.models.map((model) => {
            const beyondNative = exceedsNativeContext(model, advice.contextLength);

            return (
              <tr key={model.id} className="border-t border-edge hover:bg-white/[0.02]">
                <th scope="row" className="px-3 py-1.5 text-left font-normal">
                  <span data-selectable>{model.name}</span>
                  <span className="ml-2 font-mono text-[10px] text-neutral-600">
                    {model.parametersBillion}B
                  </span>
                  {beyondNative && (
                    <span
                      title={`This model natively accepts ${formatTokens(
                        model.architecture.maxContextLength
                      )} tokens, so this row prices a configuration it cannot be loaded at.`}
                      className="ml-2 rounded-full border border-amber-500/40 px-1.5 text-[10px] text-amber-400/80"
                    >
                      past {formatTokens(model.architecture.maxContextLength)}
                    </span>
                  )}
                </th>

                {registry.quantLevels.map((quant) => {
                  const key = cellKey(model.id, quant.id);
                  const result = index.get(key);
                  const entry = catalogue.get(key);
                  const active = selected?.model.id === model.id && selected.quant.id === quant.id;

                  return (
                    <td key={quant.id} className="px-3 py-1.5 align-top">
                      {/* Grouped so the verdict and the way to get the file are
                          one thing to a screen reader as well as to the eye:
                          the download decision is only informed if the verdict
                          travels with it. */}
                      <div
                        role="group"
                        aria-label={`${model.name} at ${quant.label}`}
                        className="flex flex-col items-start gap-1"
                      >
                        {result === undefined ? (
                          <span className="text-neutral-700">—</span>
                        ) : (
                          <button
                            type="button"
                            title={`${VERDICTS[result.verdict.kind].label}. Click for the arithmetic.`}
                            aria-label={`${model.name} at ${quant.label}: ${
                              VERDICTS[result.verdict.kind].label
                            }`}
                            onClick={() => {
                              onSelect({ model, quant, result });
                            }}
                            className={`rounded-md border px-2 py-0.5 font-mono text-[11px] transition-colors ${
                              VERDICTS[result.verdict.kind].tone
                            } ${active ? 'ring-1 ring-accent' : ''}`}
                          >
                            {VERDICTS[result.verdict.kind].short}
                          </button>
                        )}

                        <Acquisition
                          model={model}
                          quant={quant}
                          entry={entry}
                          transfer={transfer}
                          failure={failure}
                          onDownload={onDownload}
                          onPause={onPause}
                          onCancel={onCancel}
                          onRun={onRun}
                        />
                      </div>
                    </td>
                  );
                })}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

/** How a small control in a cell is styled. Repeated on six buttons otherwise. */
const CONTROL =
  'rounded-md border border-edge px-1.5 text-[10px] text-neutral-400 hover:bg-white/[0.04]';

/**
 * Which of the six states this cell is in.
 *
 * Derived in one place rather than as a chain of conditions inside the render,
 * so the states are enumerable and mutually exclusive — the property that makes
 * "downloading and downloaded at once" impossible rather than merely unlikely.
 *
 * The live `transfer` beats the catalogue, which is fetched rather than
 * streamed: between clicking Download and the next catalogue read the entry
 * still says `downloadable`.
 */
function phaseOf(
  entry: ModelCatalogueEntry | undefined,
  transfer: Transfer | null,
  failure: ModelFailure | null
): Phase {
  if (entry === undefined) return 'unpinned';

  const about = (key: ModelKey | null): boolean =>
    key !== null && key.modelId === entry.key.modelId && key.quantId === entry.key.quantId;

  if (transfer !== null && about(transfer.key)) return transfer.paused ? 'paused' : 'downloading';
  if (entry.state === 'downloaded' && entry.path !== null) return 'downloaded';
  if (failure !== null && about(failure.key) && failure.retryable) return 'failed';
  if (entry.state === 'downloading') return 'downloading';

  return 'downloadable';
}

/**
 * The controls a cell offers, one set per state it can be in.
 *
 * The six branches are the feature. An `entry` of `undefined` is a cell nobody
 * pinned, and it says so rather than rendering a control whose only outcome is
 * an error — the catalogue is a join, so a missing entry is a fact about the
 * manifest rather than a load that has not happened yet.
 *
 * Pause and Cancel both appear while downloading because they do different
 * things to the disk: Pause keeps the partial file and Cancel deletes it. A
 * single Stop would make that choice on the user's behalf, and it is not a
 * choice a progress bar should be making with several gigabytes.
 */
function Acquisition({
  model,
  quant,
  entry,
  transfer,
  failure,
  onDownload,
  onPause,
  onCancel,
  onRun,
}: {
  model: ModelEntry;
  quant: QuantLevel;
  entry: ModelCatalogueEntry | undefined;
  transfer: Transfer | null;
  failure: ModelFailure | null;
  onDownload: (key: ModelKey, totalBytes: number, from?: number) => void;
  onPause: () => void;
  onCancel: () => void;
  onRun: (entry: ModelCatalogueEntry) => void;
}): React.JSX.Element {
  const phase = phaseOf(entry, transfer, failure);
  const where = `${model.name} at ${quant.label}`;

  if (phase === 'unpinned' || entry === undefined) {
    return (
      <span
        title="No file is pinned for this cell, so there is nothing osstat could verify what it downloaded against."
        className="text-[10px] text-neutral-600"
      >
        Not pinned
      </span>
    );
  }

  if (phase === 'downloading' || phase === 'paused') {
    // Against the pinned size rather than anything a response claimed, and
    // counting whatever a resumed download already had — otherwise resuming
    // looks like starting over.
    const downloaded = transfer?.downloadedBytes ?? 0;
    const total = transfer?.totalBytes ?? entry.sizeBytes;
    const paused = phase === 'paused';

    return (
      <div className="flex w-full min-w-40 flex-col gap-1">
        <Meter
          fraction={total > 0 ? downloaded / total : 0}
          label={where}
          detail={`${formatBytes(downloaded)} of ${formatBytes(total)}`}
        />

        <Pace transfer={transfer} paused={paused} />

        <span className="flex items-center gap-1.5">
          {paused ? (
            <button
              type="button"
              aria-label={`Resume downloading ${where}`}
              onClick={() => {
                onDownload(entry.key, total, downloaded);
              }}
              className={CONTROL}
            >
              Resume
            </button>
          ) : (
            <button
              type="button"
              aria-label={`Pause downloading ${where}`}
              onClick={onPause}
              className={CONTROL}
            >
              Pause
            </button>
          )}
          <button
            type="button"
            title="Stops the download and deletes what has arrived so far. Pause keeps it."
            aria-label={`Cancel downloading ${where}`}
            onClick={onCancel}
            className={CONTROL}
          >
            Cancel
          </button>
        </span>
      </div>
    );
  }

  if (phase === 'downloaded') {
    return (
      <button
        type="button"
        aria-label={`Run ${where}`}
        title={entry.path ?? undefined}
        onClick={() => {
          onRun(entry);
        }}
        className="rounded-md border border-accent px-1.5 text-[10px] text-accent hover:bg-accent/10"
      >
        Run
      </button>
    );
  }

  if (phase === 'failed') {
    // Reached only when the payload said `retryable`, which in turn means the
    // backoff in Rust has already been through its attempts. This is the
    // manual try after the automatic ones, not a first attempt being delegated
    // to the user — and it is never offered for a wrong pin.
    return (
      <span className="flex items-center gap-1.5">
        <button
          type="button"
          title="The automatic attempts are used up. This tries again from where it stopped."
          aria-label={`Retry downloading ${where}`}
          onClick={() => {
            onDownload(entry.key, entry.sizeBytes);
          }}
          className={CONTROL}
        >
          Retry
        </button>
        <button
          type="button"
          aria-label={`Cancel downloading ${where}`}
          onClick={onCancel}
          className={CONTROL}
        >
          Cancel
        </button>
      </span>
    );
  }

  // `publisher` is only absent for a cell that was downloaded and has since
  // been unpinned, which cannot reach this branch — but the fallback says what
  // is true rather than dropping the provenance the control exists to show.
  const publisher = entry.publisher ?? 'an unnamed publisher';

  return (
    <button
      type="button"
      aria-label={`Download ${where}, ${formatBytes(entry.sizeBytes)}, via ${publisher}`}
      onClick={() => {
        onDownload(entry.key, entry.sizeBytes);
      }}
      className={CONTROL}
    >
      Download{' '}
      <span className="font-mono opacity-70">
        {formatBytes(entry.sizeBytes)} via {publisher}
      </span>
    </button>
  );
}

/**
 * Transfer rate and time remaining, when there is an honest figure for either.
 *
 * Three cases, and they are three because collapsing them would lie about one.
 * No rate yet is a window with one sample in it and says nothing. A rate of
 * zero is a stall, and it is named rather than shown as "0 B/s, 0 s left" —
 * which reads as a download about to finish. Anything else gets both figures.
 */
function Pace({
  transfer,
  paused,
}: {
  transfer: Transfer | null;
  paused: boolean;
}): React.JSX.Element | null {
  if (paused) {
    return <span className="font-mono text-[10px] text-neutral-500">Paused</span>;
  }

  const rate = transfer?.bytesPerSecond ?? null;
  if (rate === null) return null;

  if (rate === 0) {
    return (
      <span role="status" className="font-mono text-[10px] text-amber-400/80">
        Stalled
      </span>
    );
  }

  const remaining = transfer?.secondsRemaining ?? null;

  return (
    <span role="status" className="font-mono text-[10px] text-neutral-500">
      {formatRate(rate)}
      {remaining !== null && ` · ${formatDuration(remaining)} left`}
    </span>
  );
}

/**
 * What went wrong with a download, in the terms the payload distinguishes.
 *
 * Explanation only: Retry and Cancel live on the cell the failure is about, so
 * they sit beside the model they act on rather than at the top of a page that
 * may have scrolled away from it.
 *
 * A checksum mismatch is a security event rather than a bad day on the network,
 * and it is worded as one. It also gets no retry anywhere — the payload's
 * `retryable` is false, which is what keeps the control off the cell too:
 * fetching the same bytes again produces the same mismatch, and offering the
 * button invites someone to keep trying until a tampered file slips through.
 *
 * A transport failure that reaches here has already been retried automatically
 * with a bounded backoff, so the wording says the attempts are used up rather
 * than implying nothing was tried.
 */
function AcquisitionFailure({ failure }: { failure: ModelFailure }): React.JSX.Element {
  return (
    <div role="alert" className="rounded-xl border border-edge p-3 text-xs">
      <p className={failure.verificationFailure ? 'text-red-400' : 'text-amber-400/90'}>
        {failure.verificationFailure
          ? 'Verification failed. osstat removed what it downloaded and loaded nothing. The pinned checksum disagrees with the file that arrived, so fetching it again would produce the same result.'
          : failure.retryable
            ? 'The download did not finish, and the automatic attempts are used up.'
            : 'The download did not finish.'}
      </p>
      <p data-selectable className="mt-1 font-mono text-[11px] text-neutral-500">
        {failure.message}
      </p>
    </div>
  );
}

/** The arithmetic behind one verdict. */
function Drawer({
  selection,
  advice,
  onClose,
}: {
  selection: Selection;
  advice: LlmAdvice;
  onClose: () => void;
}): React.JSX.Element {
  const { model, quant, result } = selection;
  const { breakdown, verdict } = result;
  const presentation = VERDICTS[verdict.kind];

  return (
    <section
      aria-label={`How ${model.name} at ${quant.label} was worked out`}
      className="shrink-0 rounded-xl border border-edge bg-surface-raised p-3"
    >
      <div className="flex items-start justify-between gap-3">
        <div>
          <h2 className="text-sm">
            {model.name} <span className="font-mono text-neutral-500">{quant.label}</span>
          </h2>
          <p className="mt-0.5 text-xs text-neutral-400">{presentation.label}.</p>
        </div>
        <button
          type="button"
          onClick={onClose}
          aria-label="Close the explanation"
          className="rounded-md border border-edge px-2 py-0.5 text-xs text-neutral-400 hover:bg-white/[0.04]"
        >
          Close
        </button>
      </div>

      <dl className="mt-3 grid gap-x-6 gap-y-1 text-xs sm:grid-cols-2">
        <Term
          label={`Weights (${model.parametersBillion}B × ${quant.bitsPerWeight} bits ÷ 8)`}
          value={formatBytes(breakdown.quantizedWeightBytes)}
        />
        <Term label="Runtime overhead (+10%)" value={formatBytes(breakdown.overheadBytes)} />
        <Term
          label={`KV cache (${formatTokens(breakdown.contextLength)} × ${String(
            model.architecture.numLayers
          )} layers × ${String(model.architecture.numKvHeads)} KV heads × ${String(
            model.architecture.headDim
          )} × 2 × 2 bytes)`}
          value={formatBytes(breakdown.kvCacheBytes)}
        />
        <Term label="Total required" value={formatBytes(breakdown.totalRequiredBytes)} emphasis />
        <Term
          label="Weighed against VRAM"
          value={
            breakdown.availableVramBytes === 0
              ? 'none usable'
              : formatBytes(breakdown.availableVramBytes)
          }
        />
        <Term
          label="Weighed against system memory"
          value={formatBytes(breakdown.availableSystemMemoryBytes)}
        />
        <Term
          label="Layers on GPU / CPU"
          value={`${String(verdict.gpuLayers)} / ${String(verdict.cpuLayers)}`}
        />
        <Term label="Speed" value={SPEED_TIERS[verdict.tier]} />
      </dl>

      <p className="mt-3 border-t border-edge pt-2 text-[11px] text-neutral-500">
        {quant.description} The speed line is a classification, not a measurement — real throughput
        depends on the runtime, memory bandwidth and thermals, none of which osstat measures.
        Figures come from {model.sourceNote}.
      </p>

      {exceedsNativeContext(model, advice.contextLength) && (
        <p className="mt-2 text-[11px] text-amber-400/80">
          {model.name} natively accepts {formatTokens(model.architecture.maxContextLength)} tokens.
          The arithmetic above is correct for {formatTokens(advice.contextLength)}, but describes a
          configuration this model cannot be loaded at.
        </p>
      )}
    </section>
  );
}

/** One row of the arithmetic. */
function Term({
  label,
  value,
  emphasis = false,
}: {
  label: string;
  value: string;
  emphasis?: boolean;
}): React.JSX.Element {
  return (
    <div className="flex justify-between gap-3 border-b border-edge/50 py-0.5">
      <dt className="text-neutral-500">{label}</dt>
      <dd data-selectable className={`font-mono ${emphasis ? 'text-neutral-100' : ''}`}>
        {value}
      </dd>
    </div>
  );
}
