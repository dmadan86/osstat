/**
 * The LLM runnability advisor.
 *
 * Three parts, in the order ROADMAP.md M4 names them: a hardware card saying
 * what this machine has, a fit matrix of every model at every quantization,
 * and a drawer showing the arithmetic behind whichever cell you pick.
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
import type { ModelEntry } from '../bindings/ModelEntry';
import type { ModelRegistry } from '../bindings/ModelRegistry';
import type { QuantLevel } from '../bindings/QuantLevel';
import {
  budgetCaveat,
  cellKey,
  CONTEXT_LENGTHS,
  countByVerdict,
  exceedsNativeContext,
  formatTokens,
  indexResults,
  SPEED_TIERS,
  VERDICTS,
} from '../lib/advisor';
import { formatBytes } from '../lib/format';
import { fetchLlmAdvice, fetchModelRegistry, onGpusReady } from '../lib/ipc';

/** What a load can be doing. */
type LoadState =
  | { status: 'loading' }
  | { status: 'probing' }
  | { status: 'ready'; registry: ModelRegistry; advice: LlmAdvice }
  | { status: 'error'; message: string };

/** Which cell the drawer is open on. */
interface Selection {
  /** The model that row describes. */
  model: ModelEntry;
  /** The quantization that column describes. */
  quant: QuantLevel;
  /** The verdict and its arithmetic. */
  result: FitResult;
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

/** Renders the LLM runnability advisor. */
export function Llm(): React.JSX.Element {
  const [contextLength, setContextLength] = useState(4096);
  const [state, setState] = useState<LoadState>({ status: 'loading' });
  const [selected, setSelected] = useState<Selection | null>(null);

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

      {state.status === 'ready' && (
        <>
          <HardwareCard advice={state.advice} />
          <Matrix
            registry={state.registry}
            advice={state.advice}
            selected={selected}
            onSelect={setSelected}
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
  selected,
  onSelect,
}: {
  registry: ModelRegistry;
  advice: LlmAdvice;
  selected: Selection | null;
  onSelect: (selection: Selection) => void;
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
                  const result = index.get(cellKey(model.id, quant.id));
                  if (result === undefined) {
                    return (
                      <td key={quant.id} className="px-3 py-1.5 text-neutral-700">
                        —
                      </td>
                    );
                  }

                  const presentation = VERDICTS[result.verdict.kind];
                  const active = selected?.model.id === model.id && selected.quant.id === quant.id;

                  return (
                    <td key={quant.id} className="px-3 py-1.5">
                      <button
                        type="button"
                        title={`${presentation.label}. Click for the arithmetic.`}
                        aria-label={`${model.name} at ${quant.label}: ${presentation.label}`}
                        onClick={() => {
                          onSelect({ model, quant, result });
                        }}
                        className={`rounded-md border px-2 py-0.5 font-mono text-[11px] transition-colors ${
                          presentation.tone
                        } ${active ? 'ring-1 ring-accent' : ''}`}
                      >
                        {presentation.short}
                      </button>
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
