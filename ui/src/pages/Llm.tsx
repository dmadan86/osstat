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
 * Above the matrix sits a **search box**, and what it turns up is a second,
 * weaker verification tier that has to stay visibly different. A pinned model
 * is checked against a hash reviewed in a pull request against this repository.
 * A searched one is checked against the hash Hugging Face reports beside the
 * file, which catches a corrupted transfer and cannot catch a replaced upload.
 * Every searched row therefore carries `UNREVIEWED`, on the result and on the
 * model once it is downloaded — a search result that looked and downloaded
 * exactly like a pinned one would quietly retire a guarantee SECURITY.md still
 * makes.
 *
 * A searched row shows its file size, and **Check fit** reads the rest. A GGUF
 * header sits at the start of the file, so Rust fetches it with a `Range`
 * request and prices it with the same launch arithmetic a downloaded model gets
 * — the verdict is measured, not estimated, and ADR-008's rule against a figure
 * derived from size and quantization bits is kept by reading the architecture
 * rather than by declining to show one.
 *
 * That read is **per row and on demand**, never for a whole page of results: it
 * is a request against a multi-gigabyte file, and firing one for every row a
 * search returned would spend a dozen round trips on rows nobody looked at. A
 * row whose header cannot be read — an unreachable file, or a server that
 * ignores `Range` — goes back to showing its size alone and says which.
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
import type { SearchedFit } from '../bindings/SearchedFit';
import type { SearchResult } from '../bindings/SearchResult';
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
  downloadSearchedModel,
  fetchLlmAdvice,
  fetchModelCatalogue,
  fetchModelRegistry,
  onGpusReady,
  onModelDone,
  onModelFailed,
  onModelProgress,
  pauseModelDownload,
  priceSearchedModel,
  searchModels,
} from '../lib/ipc';
import { UNREVIEWED } from '../lib/provenance';
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

/**
 * Where the search has got to.
 *
 * `idle` and `empty` are separate states on purpose. Rendering nothing for both
 * would make "you have not searched yet" and "nothing matched" look identical,
 * and only one of those is worth changing the term over. `failed` is separate
 * again: "Hugging Face did not answer" is a different thing to be told, and the
 * only one of the three worth trying again.
 */
type SearchState =
  | { status: 'idle' }
  | { status: 'searching' }
  | { status: 'found'; results: SearchResult[] }
  | { status: 'empty' }
  | { status: 'failed'; message: string };

/**
 * What pricing one searched result has got to.
 *
 * A row with no entry has not been asked about — which is not the same as
 * `unpriced`, and rendering them alike would turn "nobody looked" into "osstat
 * could not tell", the one being an absence and the other a finding.
 *
 * `unpriced` carries its reason because the reasons differ in what they suggest
 * doing: a file that could not be reached is worth trying again, and a server
 * that ignores `Range` never will be.
 */
type FitState =
  | { status: 'checking' }
  | { status: 'priced'; fit: SearchedFit }
  | { status: 'unpriced'; message: string };

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
  const [term, setTerm] = useState('');
  const [found, setFound] = useState<SearchState>({ status: 'idle' });
  const [fits, setFits] = useState<Map<string, FitState>>(new Map());

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

  /**
   * Starts fetching a searched file.
   *
   * The result is handed back **exactly as it arrived**. Rust re-validates it
   * rather than trusting it, so reshaping anything here — normalising a path,
   * recomputing a name — would turn into a refusal that reads as a broken
   * button.
   */
  function downloadFound(result: SearchResult): void {
    setFailure(null);
    setRunProblem(null);
    setTransfer({
      key: { modelId: result.repo, quantId: result.file },
      downloadedBytes: 0,
      totalBytes: result.sizeBytes,
      bytesPerSecond: null,
      secondsRemaining: null,
      paused: false,
    });

    downloadSearchedModel(result).catch((error: unknown) => {
      setTransfer(null);
      setFailure({
        key: { modelId: result.repo, quantId: result.file },
        message: messageOf(error),
        retryable: false,
        verificationFailure: false,
        stopped: null,
      });
    });
  }

  /**
   * Prices one searched result by having Rust read its header.
   *
   * Called when a row is expanded and **only** then. This is a network request
   * against a file that may be thirty gigabytes, and the row it belongs to is
   * the one somebody asked about; doing it for every result a search returned
   * would spend a dozen round trips on rows nobody opened.
   *
   * Already-answered rows are left alone, so collapsing a row and opening it
   * again costs nothing. A failure is kept rather than dropped, for the same
   * reason: a row that re-fetched on every open would ask an unreachable host
   * again on each one.
   */
  function checkFit(result: SearchResult): void {
    const cell = cellKey(result.repo, result.file);
    if (fits.has(cell)) return;

    setFits((held) => new Map(held).set(cell, { status: 'checking' }));

    priceSearchedModel(result).then(
      (fit) => {
        setFits((held) => new Map(held).set(cell, { status: 'priced', fit }));
      },
      (error: unknown) => {
        setFits((held) =>
          new Map(held).set(cell, { status: 'unpriced', message: messageOf(error) })
        );
      }
    );
  }

  /** Runs the search, or reports why it could not be made. */
  function runSearch(): void {
    const query = term.trim();
    if (query === '') return;

    setFound({ status: 'searching' });
    // A new search replaces the rows, so verdicts belonging to the old ones
    // would be answers to questions no longer on screen -- and a cell key
    // repeated across two searches would show the earlier search's verdict.
    setFits(new Map());
    searchModels(query).then(
      (results) => {
        setFound(results.length === 0 ? { status: 'empty' } : { status: 'found', results });
      },
      (error: unknown) => {
        setFound({ status: 'failed', message: messageOf(error) });
      }
    );
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
          className="rounded-xl border border-edge p-6 text-center text-sm text-text-muted"
        >
          Reading the model registry…
        </p>
      )}

      {state.status === 'probing' && (
        <p
          role="status"
          className="rounded-xl border border-edge p-6 text-center text-sm text-text-muted"
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

      <SearchPanel
        term={term}
        state={found}
        catalogue={entries}
        transfer={transfer}
        onTerm={setTerm}
        onSearch={runSearch}
        onDownload={downloadFound}
        onPause={pause}
        onCancel={cancel}
        onRun={run}
        fits={fits}
        onCheckFit={checkFit}
      />

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
      <span className="text-xs text-text-muted">Context length</span>
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
                : 'border-edge text-text-muted hover:bg-white/[0.04]'
            }`}
          >
            {formatTokens(tokens)}
          </button>
        ))}
      </div>
      <p className="text-[11px] text-text-faint">
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
      <span className="text-[10px] uppercase tracking-wider text-text-muted">{label}</span>
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
          <tr className="text-[10px] uppercase tracking-wider text-text-muted">
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
                  <span className="ml-2 font-mono text-[10px] text-text-faint">
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
                          <span className="text-text-faint">—</span>
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

/**
 * The whole of that argument, for the control's tooltip.
 *
 * Page-specific and therefore not alongside {@link UNREVIEWED} in
 * `lib/provenance`: it points at the pinned matrix drawn above it, which exists
 * on this page and nowhere else.
 */
const UNREVIEWED_DETAIL =
  'osstat checks this against the hash Hugging Face reports beside the file. That catches a corrupted transfer. It cannot show that the upload is the one anybody reviewed, which is what the pinned models above are checked against.';

/** How a small control in a cell is styled. Repeated on six buttons otherwise. */
const CONTROL =
  'rounded-md border border-edge px-1.5 text-[10px] text-text-muted hover:bg-white/[0.04]';

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
        className="text-[10px] text-text-faint"
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
 * The search box, whatever it turned up, and anything already fetched with it.
 *
 * Its own section, above the fit matrix and visibly apart from it, because the
 * two lists carry different guarantees. Everything in the matrix was pinned in
 * a reviewed pull request; everything here is whatever the search returned.
 *
 * **A result shows its file size until somebody asks for more.** Check fit has
 * Rust read the GGUF header off the front of the file with a `Range` request and
 * price it with the launch arithmetic a downloaded model gets, so the verdict is
 * the real one rather than a figure derived from size and quantization bits —
 * which is what ADR-008 names as the worst thing this feature could do.
 *
 * The read is per row and only when a row is opened. One request against a
 * multi-gigabyte file is cheap; a dozen fired at a page of results nobody
 * expanded is not.
 *
 * The downloaded list below the results is not redundant with them: the results
 * live only as long as the term does, and a model's tier has to outlive the
 * search that found it or a restart would lose it. Those rows offer no Check fit
 * — a downloaded model is priced from the file itself the moment it is Run.
 */
function SearchPanel({
  term,
  state,
  catalogue,
  transfer,
  onTerm,
  onSearch,
  onDownload,
  onPause,
  onCancel,
  onRun,
  fits,
  onCheckFit,
}: {
  term: string;
  state: SearchState;
  catalogue: Map<string, ModelCatalogueEntry>;
  transfer: Transfer | null;
  onTerm: (term: string) => void;
  onSearch: () => void;
  onDownload: (result: SearchResult) => void;
  onPause: () => void;
  onCancel: () => void;
  onRun: (entry: ModelCatalogueEntry) => void;
  fits: Map<string, FitState>;
  onCheckFit: (result: SearchResult) => void;
}): React.JSX.Element {
  const shown = state.status === 'found' ? state.results : [];
  const alreadyShown = new Set(shown.map((result) => cellKey(result.repo, result.file)));

  // Everything fetched by search that is not in front of us already. Read from
  // the catalogue rather than remembered here, so it survives a reload.
  const downloaded = [...catalogue.values()].filter(
    (entry) =>
      entry.provenance === 'searched' &&
      entry.state === 'downloaded' &&
      !alreadyShown.has(cellKey(entry.key.modelId, entry.key.quantId))
  );

  return (
    <section
      aria-label="Found on Hugging Face"
      className="shrink-0 rounded-xl border border-edge bg-surface-raised p-3"
    >
      <form
        className="flex flex-wrap items-center gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          onSearch();
        }}
      >
        <input
          type="search"
          aria-label="Search Hugging Face for a model"
          placeholder="Search Hugging Face…"
          value={term}
          onChange={(event) => {
            onTerm(event.target.value);
          }}
          className="min-w-56 flex-1 rounded-md border border-edge bg-transparent px-2 py-1 text-xs text-text placeholder:text-text-faint"
        />
        <button type="submit" className={CONTROL}>
          Search
        </button>
      </form>

      <p className="mt-2 text-[11px] text-text-muted">
        Anything found here is checked against a hash Hugging Face reports beside the file, which is
        a weaker promise than the pinned models below carry. Check fit reads the header off the
        front of a file without downloading it, and prices it exactly as a downloaded model is
        priced.
      </p>

      {state.status === 'searching' && (
        <p role="status" className="mt-2 text-xs text-text-muted">
          Searching…
        </p>
      )}

      {state.status === 'empty' && (
        <p role="status" className="mt-2 text-xs text-text-muted">
          Nothing on Hugging Face matched that. Only whole, hashed GGUF files are offered, so a
          repository holding a split model or no GGUF at all will not appear.
        </p>
      )}

      {state.status === 'failed' && (
        <p role="alert" className="mt-2 text-xs text-amber-400/90">
          The search could not be made: {state.message}
        </p>
      )}

      {shown.length > 0 && (
        <ul className="mt-2 flex flex-col gap-1.5">
          {shown.map((result) => (
            <li key={cellKey(result.repo, result.file)}>
              <FoundRow
                name={result.file}
                cell={cellKey(result.repo, result.file)}
                publisher={result.publisher}
                sizeBytes={result.sizeBytes}
                quantHint={result.quantHint}
                entry={catalogue.get(cellKey(result.repo, result.file))}
                transfer={transfer}
                onDownload={() => {
                  onDownload(result);
                }}
                onPause={onPause}
                onCancel={onCancel}
                onRun={onRun}
                fit={fits.get(cellKey(result.repo, result.file))}
                onCheckFit={() => {
                  onCheckFit(result);
                }}
              />
            </li>
          ))}
        </ul>
      )}

      {downloaded.length > 0 && (
        <ul className="mt-2 flex flex-col gap-1.5 border-t border-edge pt-2">
          {downloaded.map((entry) => (
            <li key={cellKey(entry.key.modelId, entry.key.quantId)}>
              <FoundRow
                name={entry.file ?? entry.key.quantId}
                cell={cellKey(entry.key.modelId, entry.key.quantId)}
                publisher={entry.publisher ?? 'an unnamed publisher'}
                sizeBytes={entry.sizeBytes}
                quantHint={null}
                entry={entry}
                transfer={transfer}
                onDownload={() => {
                  // Already on disk; this row never offers a download.
                }}
                onPause={onPause}
                onCancel={onCancel}
                onRun={onRun}
              />
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/**
 * One searched file: what it is, what it weighs, and which tier fetched it.
 *
 * The label rides on the row rather than on the section heading, so it stays
 * attached to the model after the search that found it is gone — and so a
 * screen reader hears it as part of the same group as the control, which is the
 * only arrangement in which it can inform the decision it is there to inform.
 *
 * `onCheckFit` is absent for a row that came from the catalogue rather than from
 * a search: a downloaded model is priced from the file on disk when it is Run,
 * and fetching its header over the network to say the same thing would be a
 * request for nothing.
 */
function FoundRow({
  name,
  cell,
  publisher,
  sizeBytes,
  quantHint,
  entry,
  transfer,
  onDownload,
  onPause,
  onCancel,
  onRun,
  fit,
  onCheckFit,
}: {
  name: string;
  cell: string;
  publisher: string;
  sizeBytes: number;
  quantHint: string | null;
  entry: ModelCatalogueEntry | undefined;
  transfer: Transfer | null;
  onDownload: () => void;
  onPause: () => void;
  onCancel: () => void;
  onRun: (entry: ModelCatalogueEntry) => void;
  fit?: FitState | undefined;
  onCheckFit?: (() => void) | undefined;
}): React.JSX.Element {
  const [open, setOpen] = useState(false);

  return (
    <div
      role="group"
      aria-label={name}
      className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-md border border-edge px-2 py-1.5"
    >
      <span data-selectable className="font-mono text-[11px] text-text">
        {name}
      </span>
      <span className="font-mono text-[11px] text-text-muted">{formatBytes(sizeBytes)}</span>
      <span className="text-[11px] text-text-muted">via {publisher}</span>
      {quantHint !== null && (
        <span className="rounded-full border border-edge px-1.5 font-mono text-[10px] text-text-muted">
          {quantHint}
        </span>
      )}
      <span
        title={UNREVIEWED_DETAIL}
        className="rounded-full border border-amber-500/40 px-1.5 text-[10px] text-amber-400/90"
      >
        {UNREVIEWED}
      </span>

      {onCheckFit !== undefined && (
        <button
          type="button"
          aria-expanded={open}
          aria-label={`Check fit for ${name}`}
          onClick={() => {
            // The fetch is fired on the way open and never on the way shut, and
            // `onCheckFit` ignores a row it has already answered — so this
            // costs one request per row however many times it is toggled.
            if (!open) onCheckFit();
            setOpen(!open);
          }}
          className="rounded-md border border-edge px-1.5 text-[10px] text-text-muted hover:bg-white/[0.04]"
        >
          Check fit
        </button>
      )}

      <span className="ml-auto">
        <FoundControl
          name={name}
          cell={cell}
          publisher={publisher}
          sizeBytes={sizeBytes}
          entry={entry}
          transfer={transfer}
          onDownload={onDownload}
          onPause={onPause}
          onCancel={onCancel}
          onRun={onRun}
        />
      </span>

      {open && (
        <span className="w-full border-t border-edge pt-1.5">
          <FoundFit fit={fit} name={name} />
        </span>
      )}
    </div>
  );
}

/**
 * The verdict for one searched row, or why there is not one.
 *
 * Every figure here came out of the same `plan_launch` a downloaded model is
 * opened with, over a header read from the front of the actual file. Nothing is
 * derived from the file size, so the wording can be the plain wording the
 * session banner uses rather than a hedged version of it — an estimate dressed
 * as a measurement is the failure ADR-008 names, and the way to avoid it is to
 * measure, which is what happened.
 *
 * An unpriced row falls back to the size the search reported and says which of
 * the reasons applied. It never shows a partial verdict: half an answer here
 * would be the guess the whole design refuses.
 */
function FoundFit({ fit, name }: { fit: FitState | undefined; name: string }): React.JSX.Element {
  if (fit === undefined || fit.status === 'checking') {
    return (
      <span role="status" className="text-[11px] text-text-muted">
        Checking the fit… osstat is reading this file&rsquo;s header without downloading it.
      </span>
    );
  }

  if (fit.status === 'unpriced') {
    return (
      <span className="text-[11px] text-amber-400/80">
        The header could not be read, so {name} shows its size and nothing more: {fit.message}
      </span>
    );
  }

  const { gpuLayers, blockCount, contextLength, fits, headDimDerived } = fit.fit;

  return (
    <span className="flex flex-col gap-1 text-[11px]">
      <span className={fits ? 'text-emerald-400/90' : 'text-amber-400/90'}>
        {fits
          ? `Fits entirely in VRAM: all ${String(blockCount)} layers on GPU.`
          : `${String(gpuLayers)} of ${String(blockCount)} layers on GPU, the rest on the CPU.`}
      </span>
      <span className="text-text-muted">
        Context {formatTokens(contextLength)}. Read from this file&rsquo;s own header and priced by
        the same arithmetic the pinned models use — not estimated from its size.
      </span>
      {!fits && (
        <span className="text-text-muted">
          Generation will be slower. The figure is an estimate, which is why this is a warning
          rather than a refusal.
        </span>
      )}
      {headDimDerived && (
        <span className="text-text-faint">
          This model&rsquo;s header declares no attention key length, so the KV-cache arithmetic
          derived one. That is correct for standard attention and wrong for models that diverge.
        </span>
      )}
    </span>
  );
}

/**
 * The one control a searched row offers, given what it is currently doing.
 *
 * Early returns rather than nested conditions, for the same reason
 * {@link phaseOf} exists above: the states are enumerable and mutually
 * exclusive, so "downloading and downloaded at once" is impossible rather than
 * merely unlikely.
 *
 * The download hands the result straight back to Rust. The progress branch
 * measures against the size the search reported, not against anything a
 * response claimed — the same rule the pinned matrix follows.
 */
function FoundControl({
  name,
  cell,
  publisher,
  sizeBytes,
  entry,
  transfer,
  onDownload,
  onPause,
  onCancel,
  onRun,
}: {
  name: string;
  cell: string;
  publisher: string;
  sizeBytes: number;
  entry: ModelCatalogueEntry | undefined;
  transfer: Transfer | null;
  onDownload: () => void;
  onPause: () => void;
  onCancel: () => void;
  onRun: (entry: ModelCatalogueEntry) => void;
}): React.JSX.Element {
  if (entry !== undefined && entry.state === 'downloaded' && entry.path !== null) {
    const path = entry.path;
    return (
      <button
        type="button"
        aria-label={`Run ${name}`}
        title={path}
        onClick={() => {
          onRun(entry);
        }}
        className="rounded-md border border-accent px-1.5 text-[10px] text-accent hover:bg-accent/10"
      >
        Run
      </button>
    );
  }

  if (transfer !== null && cellKey(transfer.key.modelId, transfer.key.quantId) === cell) {
    return (
      <span className="flex w-40 flex-col gap-1">
        <Meter
          fraction={sizeBytes > 0 ? transfer.downloadedBytes / sizeBytes : 0}
          label={name}
          detail={`${formatBytes(transfer.downloadedBytes)} of ${formatBytes(sizeBytes)}`}
        />
        <span className="flex items-center gap-1.5">
          <button
            type="button"
            aria-label={`Pause downloading ${name}`}
            onClick={onPause}
            className={CONTROL}
          >
            Pause
          </button>
          <button
            type="button"
            aria-label={`Cancel downloading ${name}`}
            onClick={onCancel}
            className={CONTROL}
          >
            Cancel
          </button>
        </span>
      </span>
    );
  }

  return (
    <button
      type="button"
      aria-label={`Download ${name}, ${formatBytes(sizeBytes)}, via ${publisher}, ${UNREVIEWED}`}
      onClick={onDownload}
      className={CONTROL}
    >
      Download
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
    return <span className="font-mono text-[10px] text-text-muted">Paused</span>;
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
    <span role="status" className="font-mono text-[10px] text-text-muted">
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
      <p data-selectable className="mt-1 font-mono text-[11px] text-text-muted">
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
            {model.name} <span className="font-mono text-text-muted">{quant.label}</span>
          </h2>
          <p className="mt-0.5 text-xs text-text-muted">{presentation.label}.</p>
        </div>
        <button
          type="button"
          onClick={onClose}
          aria-label="Close the explanation"
          className="rounded-md border border-edge px-2 py-0.5 text-xs text-text-muted hover:bg-white/[0.04]"
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

      <p className="mt-3 border-t border-edge pt-2 text-[11px] text-text-muted">
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
      <dt className="text-text-muted">{label}</dt>
      <dd data-selectable className={`font-mono ${emphasis ? 'text-text' : ''}`}>
        {value}
      </dd>
    </div>
  );
}
