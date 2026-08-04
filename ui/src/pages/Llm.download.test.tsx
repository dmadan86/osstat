/**
 * The acquisition controls in the fit matrix.
 *
 * Every test here asserts on **content**, not presence. A test that only
 * checked "a button exists" would pass with Download and Run swapped, and with
 * the publisher's name dropped from the control that exists to show it — the
 * exact defect class found twice already in this repo. So the download test
 * names bartowski, the run test names the path `chat_open_model` was given, and
 * the unpinned test asserts the cell *says so* rather than merely lacking a
 * button.
 *
 * `../lib/ipc` is mocked the way `Settings.runtime.test.tsx` does it: the
 * subscription functions capture the handler they were given and a test fires
 * it. Nothing here makes a request or writes a file.
 */

import { act, render, screen, waitFor, within } from '@testing-library/react';
import { useState } from 'react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { LlmAdvice } from '../bindings/LlmAdvice';
import type { ModelCatalogueEntry } from '../bindings/ModelCatalogueEntry';
import type { ModelDone } from '../bindings/ModelDone';
import type { ModelFailure } from '../bindings/ModelFailure';
import type { ModelProgress } from '../bindings/ModelProgress';
import type { ModelRegistry } from '../bindings/ModelRegistry';
import type { ModelSession } from '../bindings/ModelSession';

const {
  cancelModelDownload,
  chatOpenModel,
  chatStatus,
  downloadModel,
  fetchLlmAdvice,
  fetchModelCatalogue,
  fetchModelRegistry,
  onGpusReady,
  onModelDone,
  onModelFailed,
  onModelProgress,
  pauseModelDownload,
} = vi.hoisted(() => ({
  cancelModelDownload: vi.fn(),
  chatOpenModel: vi.fn(),
  chatStatus: vi.fn(),
  downloadModel: vi.fn(),
  fetchLlmAdvice: vi.fn(),
  fetchModelCatalogue: vi.fn(),
  fetchModelRegistry: vi.fn(),
  onGpusReady: vi.fn(),
  onModelDone: vi.fn(),
  onModelFailed: vi.fn(),
  onModelProgress: vi.fn(),
  pauseModelDownload: vi.fn(),
}));

vi.mock('../lib/ipc', () => ({
  cancelModelDownload,
  chatOpenModel,
  chatStatus,
  downloadModel,
  fetchLlmAdvice,
  fetchModelCatalogue,
  fetchModelRegistry,
  onGpusReady,
  onModelDone,
  onModelFailed,
  onModelProgress,
  pauseModelDownload,
}));

const { Llm } = await import('./Llm');

/** Captures the handler a subscription was given, so a test can fire it. */
function capturing<T>(): {
  fire: (payload: T) => void;
  subscribe: (handler: (payload: T) => void) => Promise<() => void>;
} {
  let held: ((payload: T) => void) | null = null;

  return {
    fire: (payload) => held?.(payload),
    subscribe: (handler) => {
      held = handler;
      return Promise.resolve(() => {
        held = null;
      });
    },
  };
}

const progress = capturing<ModelProgress>();
const done = capturing<ModelDone>();
const failed = capturing<ModelFailure>();

/** The pinned file's real size, so the progress readout is a real fraction. */
const LLAMA_BYTES = 4_920_734_688;

/** Where a downloaded Llama would live, as the record spells it. */
const LLAMA_PATH = 'D:\\models\\Meta-Llama-3-8B-Instruct-Q4_K_M.gguf';

function registry(): ModelRegistry {
  return {
    version: 1,
    quantLevels: [
      { id: 'Q4_K_M', label: 'Q4_K_M', bitsPerWeight: 4.85, description: 'A good default.' },
      { id: 'Q8_0', label: 'Q8_0', bitsPerWeight: 8.5, description: 'Nearly lossless.' },
    ],
    models: [
      {
        id: 'llama-3-8b',
        name: 'Llama 3 8B',
        family: 'Llama',
        parametersBillion: 8,
        architecture: {
          numLayers: 32,
          hiddenSize: 4096,
          numAttentionHeads: 32,
          numKvHeads: 8,
          headDim: 128,
          maxContextLength: 8192,
        },
        sourceNote: 'the model card',
        downloads: [],
      },
      {
        id: 'qwen2.5-72b',
        name: 'Qwen2.5 72B',
        family: 'Qwen',
        parametersBillion: 72,
        architecture: {
          numLayers: 80,
          hiddenSize: 8192,
          numAttentionHeads: 64,
          numKvHeads: 8,
          headDim: 128,
          maxContextLength: 32768,
        },
        sourceNote: 'the model card',
        downloads: [],
      },
    ],
  };
}

function advice(): LlmAdvice {
  return {
    gpu: { present: true, vramBytes: 12_000_000_000 },
    systemMemoryBytes: 32_000_000_000,
    contextLength: 4096,
    results: [
      {
        modelId: 'llama-3-8b',
        quantId: 'Q4_K_M',
        verdict: { kind: 'fitsOnGpu', gpuLayers: 32, cpuLayers: 0, tier: 'fast' },
        breakdown: {
          quantizedWeightBytes: 4_850_000_000,
          overheadBytes: 485_000_000,
          kvCacheBytes: 1_073_741_824,
          totalRequiredBytes: 6_408_741_824,
          availableVramBytes: 12_000_000_000,
          availableSystemMemoryBytes: 32_000_000_000,
          contextLength: 4096,
        },
      },
      {
        modelId: 'llama-3-8b',
        quantId: 'Q8_0',
        verdict: { kind: 'fitsWithCpuOffload', gpuLayers: 20, cpuLayers: 12, tier: 'moderate' },
        breakdown: {
          quantizedWeightBytes: 8_500_000_000,
          overheadBytes: 850_000_000,
          kvCacheBytes: 1_073_741_824,
          totalRequiredBytes: 10_423_741_824,
          availableVramBytes: 12_000_000_000,
          availableSystemMemoryBytes: 32_000_000_000,
          contextLength: 4096,
        },
      },
      {
        modelId: 'qwen2.5-72b',
        quantId: 'Q4_K_M',
        verdict: { kind: 'wontFit', gpuLayers: 0, cpuLayers: 80, tier: 'slow' },
        breakdown: {
          quantizedWeightBytes: 43_650_000_000,
          overheadBytes: 4_365_000_000,
          kvCacheBytes: 2_684_354_560,
          totalRequiredBytes: 50_699_354_560,
          availableVramBytes: 12_000_000_000,
          availableSystemMemoryBytes: 32_000_000_000,
          contextLength: 4096,
        },
      },
    ],
  };
}

/**
 * The catalogue: Llama at Q4_K_M and the 72B pinned, Llama at Q8_0 not.
 *
 * The gap is the point. A cell with no entry is one nobody pinned, and that is
 * what the "not pinned" branch renders from.
 */
function catalogue(overrides: Partial<ModelCatalogueEntry> = {}): ModelCatalogueEntry[] {
  return [
    {
      key: { modelId: 'llama-3-8b', quantId: 'Q4_K_M' },
      state: 'downloadable',
      publisher: 'bartowski',
      repo: 'bartowski/Meta-Llama-3-8B-Instruct-GGUF',
      file: 'Meta-Llama-3-8B-Instruct-Q4_K_M.gguf',
      sizeBytes: LLAMA_BYTES,
      path: null,
      provenance: 'pinned',
      ...overrides,
    },
    {
      key: { modelId: 'qwen2.5-72b', quantId: 'Q4_K_M' },
      state: 'downloadable',
      publisher: 'bartowski',
      repo: 'bartowski/Qwen2.5-72B-Instruct-GGUF',
      file: 'Qwen2.5-72B-Instruct-Q4_K_M.gguf',
      sizeBytes: 47_415_707_136,
      path: null,
      provenance: 'pinned',
    },
  ];
}

function session(): ModelSession {
  return {
    modelName: 'Meta-Llama-3-8B-Instruct-Q4_K_M',
    gpuLayers: 32,
    contextLength: 8192,
    fits: true,
    headDimDerived: false,
    vision: false,
  };
}

/** The controls for one cell, addressed by what the cell describes. */
async function cellOf(model: string, quant: string): Promise<HTMLElement> {
  return screen.findByRole('group', { name: `${model} at ${quant}` });
}

/**
 * Starts the Llama download and drives it to `downloadedBytes`.
 *
 * The rate and estimate are the ones the backend computes over its own window;
 * the front end renders what it is given rather than deriving them again, so a
 * fixture is exactly as good as the real figure here.
 */
async function downloading(
  downloadedBytes = 2_460_367_344,
  pace: Pick<ModelProgress, 'bytesPerSecond' | 'secondsRemaining'> = {
    bytesPerSecond: 12_582_912,
    secondsRemaining: 196,
  }
): Promise<HTMLElement> {
  const user = userEvent.setup();
  const cell = await cellOf('Llama 3 8B', 'Q4_K_M');
  await user.click(within(cell).getByRole('button', { name: /^download /i }));

  act(() => {
    progress.fire({
      key: { modelId: 'llama-3-8b', quantId: 'Q4_K_M' },
      phase: 'downloading',
      downloadedBytes,
      totalBytes: LLAMA_BYTES,
      ...pace,
    });
  });

  return cell;
}

beforeEach(() => {
  vi.clearAllMocks();
  onGpusReady.mockResolvedValue(() => undefined);
  onModelProgress.mockImplementation(progress.subscribe);
  onModelDone.mockImplementation(done.subscribe);
  onModelFailed.mockImplementation(failed.subscribe);
  fetchModelRegistry.mockResolvedValue(registry());
  fetchLlmAdvice.mockResolvedValue(advice());
  fetchModelCatalogue.mockResolvedValue(catalogue());
  downloadModel.mockResolvedValue(undefined);
  pauseModelDownload.mockResolvedValue(undefined);
  cancelModelDownload.mockResolvedValue(undefined);
  chatOpenModel.mockResolvedValue(session());
  chatStatus.mockResolvedValue(null);
});

describe('Llm › acquiring a model', () => {
  it('names the publisher on the download control', async () => {
    // These are community re-quantizations, not the model vendors' own
    // uploads. Trusting a third party is a real trade, so the control that
    // makes it says whose file it is rather than leaving that in the manifest.
    render(<Llm />);

    const cell = await cellOf('Llama 3 8B', 'Q4_K_M');
    const download = within(cell).getByRole('button', { name: /^download /i });

    expect(download).toHaveTextContent(/via bartowski/i);
    expect(download).toHaveAccessibleName(/via bartowski/i);
  });

  it('states the pinned size on the download control', async () => {
    // 4.58 GB is a decision the user makes before it starts, not a surprise
    // they discover from a progress bar.
    render(<Llm />);

    const cell = await cellOf('Llama 3 8B', 'Q4_K_M');

    expect(within(cell).getByRole('button', { name: /^download /i })).toHaveTextContent(/4\.58 GB/);
  });

  it('shows progress against the pinned size while downloading', async () => {
    render(<Llm />);

    const cell = await downloading();

    expect(downloadModel).toHaveBeenCalledWith('llama-3-8b', 'Q4_K_M');

    // Both figures, in the right order. A meter that showed only one, or that
    // swapped them, would still have "a progress readout".
    expect(cell).toHaveTextContent(/2\.29 GB of 4\.58 GB/);
    expect(within(cell).queryByRole('button', { name: /^download /i })).toBeNull();

    // And only the cell actually downloading: a bar on every row at once would
    // say osstat is fetching three models when it is fetching one.
    const other = await cellOf('Qwen2.5 72B', 'Q4_K_M');
    expect(other).not.toHaveTextContent(/2\.29 GB/);
    expect(within(other).getByRole('button', { name: /^download /i })).toBeInTheDocument();
  });

  it('shows a progress bar with the fraction downloaded', async () => {
    // The bar is the feature, and the fraction is what makes it a bar rather
    // than a decoration. `Meter` is the component the Overview draws CPU and
    // RAM with, so this is the same control the rest of the app uses.
    render(<Llm />);

    // Exactly half of the pinned size, so a bar reading anything but 50 is
    // measuring against something it should not be.
    const cell = await downloading(LLAMA_BYTES / 2);
    const bar = within(cell).getByRole('meter');

    expect(bar).toHaveAttribute('aria-valuenow', '50');
    expect(bar).toHaveAttribute('aria-valuemin', '0');
    expect(bar).toHaveAttribute('aria-valuemax', '100');
  });

  it('shows transfer rate and estimated time while downloading', async () => {
    // A bar with no rate cannot answer "should I wait for this?", which is the
    // only question anyone asks a download.
    render(<Llm />);

    const cell = await downloading(2_460_367_344, {
      bytesPerSecond: 12_582_912,
      secondsRemaining: 196,
    });

    expect(cell).toHaveTextContent(/12\.0 MB\/s/);
    expect(cell).toHaveTextContent(/3 m/);
  });

  it('says nothing about the rate until there is a rate to report', async () => {
    // The first progress event has nothing to measure against. A "0 B/s" there
    // would say the download had stalled the instant it started.
    render(<Llm />);

    const cell = await downloading(0, { bytesPerSecond: null, secondsRemaining: null });

    expect(cell).not.toHaveTextContent(/\/s/);
    expect(cell).not.toHaveTextContent(/left/i);
  });

  it('shows a stall as a stall rather than as a stale estimate', async () => {
    // The backend reports a zero rate and no estimate for a transfer that has
    // stopped moving. Carrying on showing the last estimate would be a number
    // that quietly stopped meaning anything.
    render(<Llm />);

    const cell = await downloading(2_460_367_344, {
      bytesPerSecond: 0,
      secondsRemaining: null,
    });

    expect(cell).toHaveTextContent(/stalled/i);
    expect(cell).not.toHaveTextContent(/left/i);
  });

  it('offers Pause while downloading and Resume once paused', async () => {
    const user = userEvent.setup();
    render(<Llm />);

    const cell = await downloading();

    // Asserted on the visible word as well as the accessible name: a test that
    // checked only that "a button exists" would pass with the two swapped.
    const pause = within(cell).getByRole('button', { name: /^pause /i });
    expect(pause).toHaveTextContent(/^Pause$/);
    expect(within(cell).queryByRole('button', { name: /^resume /i })).toBeNull();

    await user.click(pause);
    expect(pauseModelDownload).toHaveBeenCalledTimes(1);

    act(() => {
      failed.fire({
        key: { modelId: 'llama-3-8b', quantId: 'Q4_K_M' },
        message: 'the download was paused; resuming continues from where it stopped',
        retryable: true,
        verificationFailure: false,
        stopped: 'pause',
      });
    });

    const resume = within(cell).getByRole('button', { name: /^resume /i });
    expect(resume).toHaveTextContent(/^Resume$/);
    expect(within(cell).queryByRole('button', { name: /^pause /i })).toBeNull();

    await user.click(resume);
    expect(downloadModel).toHaveBeenLastCalledWith('llama-3-8b', 'Q4_K_M');
  });

  it('keeps the progress figure across a pause, so nothing looks lost', async () => {
    // The partial file is kept on purpose. A bar that dropped to zero would
    // say the 2.29 GB already on disk had been thrown away.
    render(<Llm />);

    const cell = await downloading(2_460_367_344);
    expect(cell).toHaveTextContent(/2\.29 GB of 4\.58 GB/);

    act(() => {
      failed.fire({
        key: { modelId: 'llama-3-8b', quantId: 'Q4_K_M' },
        message: 'the download was paused; resuming continues from where it stopped',
        retryable: true,
        verificationFailure: false,
        stopped: 'pause',
      });
    });

    expect(cell).toHaveTextContent(/2\.29 GB of 4\.58 GB/);
    expect(within(cell).getByRole('meter')).toHaveAttribute('aria-valuenow', '50');
  });

  it('forgets the progress figure on a cancel, because those bytes are gone', async () => {
    // The mirror of the pause test, and the reason the payload distinguishes
    // the two: after a cancel the partial file has been deleted, so a bar
    // still reading 2.29 GB would promise a resume that would start from zero.
    render(<Llm />);

    const cell = await downloading(2_460_367_344);

    act(() => {
      failed.fire({
        key: { modelId: 'llama-3-8b', quantId: 'Q4_K_M' },
        message: 'the download was cancelled; what had already arrived was deleted',
        retryable: true,
        verificationFailure: false,
        stopped: 'cancel',
      });
    });

    expect(cell).not.toHaveTextContent(/2\.29 GB/);
    expect(within(cell).queryByRole('meter')).toBeNull();
    expect(within(cell).getByRole('button', { name: /^download /i })).toBeInTheDocument();
  });

  it('says a permanent failure is a bad pin, not a network problem', async () => {
    // The user-visible half of the retry policy. "Download failed, retrying"
    // for a wrong hash would be a lie, and it would send someone to check
    // their wifi over what is a supply-chain event.
    render(<Llm />);
    await cellOf('Llama 3 8B', 'Q4_K_M');

    act(() => {
      failed.fire({
        key: { modelId: 'llama-3-8b', quantId: 'Q4_K_M' },
        message: 'Meta-Llama-3-8B-Instruct-Q4_K_M.gguf did not match its pinned checksum',
        retryable: false,
        verificationFailure: true,
        stopped: null,
      });
    });

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent(/verification failed/i);
    expect(alert).not.toHaveTextContent(/network/i);
    expect(alert).not.toHaveTextContent(/retrying/i);
    expect(alert).not.toHaveTextContent(/connection/i);
    // And nowhere on the page, not merely nowhere in the alert: a Retry in the
    // cell would invite exactly the loop the policy exists to prevent.
    expect(screen.queryByRole('button', { name: /^retry /i })).toBeNull();
  });

  it('offers Retry after the attempts are exhausted', async () => {
    // A `model:failed` carrying `retryable` has already been through the
    // backoff in Rust, so this control is the manual attempt after the
    // automatic ones — not a first try the user is being asked to make.
    const user = userEvent.setup();
    render(<Llm />);
    await downloading();

    act(() => {
      failed.fire({
        key: { modelId: 'llama-3-8b', quantId: 'Q4_K_M' },
        message: 'could not fetch the file: connection reset',
        retryable: true,
        verificationFailure: false,
        stopped: null,
      });
    });

    const cell = await cellOf('Llama 3 8B', 'Q4_K_M');
    const retry = within(cell).getByRole('button', { name: /^retry /i });
    expect(retry).toHaveTextContent(/^Retry$/);

    await user.click(retry);
    expect(downloadModel).toHaveBeenLastCalledWith('llama-3-8b', 'Q4_K_M');
  });

  it('offers Cancel alongside Retry, so a failure can be cleared', async () => {
    const user = userEvent.setup();
    render(<Llm />);
    await downloading();

    act(() => {
      failed.fire({
        key: { modelId: 'llama-3-8b', quantId: 'Q4_K_M' },
        message: 'could not fetch the file: connection reset',
        retryable: true,
        verificationFailure: false,
        stopped: null,
      });
    });

    const cell = await cellOf('Llama 3 8B', 'Q4_K_M');
    await user.click(within(cell).getByRole('button', { name: /^cancel /i }));

    expect(within(cell).getByRole('button', { name: /^download /i })).toBeInTheDocument();
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('offers Run once downloaded', async () => {
    // The last of the six states, and the one the other five exist to reach.
    fetchModelCatalogue.mockResolvedValue(catalogue({ state: 'downloaded', path: LLAMA_PATH }));
    render(<Llm />);

    const cell = await cellOf('Llama 3 8B', 'Q4_K_M');

    expect(within(cell).getByRole('button', { name: /^run /i })).toHaveTextContent(/^Run$/);
    expect(within(cell).queryByRole('meter')).toBeNull();
    expect(within(cell).queryByRole('button', { name: /^download /i })).toBeNull();
  });

  it('offers Run once downloaded, and opens the chat with that path', async () => {
    // The same command the file picker uses, given the record's own absolute
    // path — one path into a session rather than two.
    const user = userEvent.setup();
    const onModelOpened = vi.fn();
    fetchModelCatalogue.mockResolvedValue(catalogue({ state: 'downloaded', path: LLAMA_PATH }));
    render(<Llm onModelOpened={onModelOpened} />);

    const cell = await cellOf('Llama 3 8B', 'Q4_K_M');
    expect(within(cell).queryByRole('button', { name: /^download /i })).toBeNull();

    await user.click(within(cell).getByRole('button', { name: /^run llama 3 8b at q4_k_m$/i }));

    await waitFor(() => {
      expect(chatOpenModel).toHaveBeenCalledWith(LLAMA_PATH);
    });
    await waitFor(() => {
      expect(onModelOpened).toHaveBeenCalledWith(session());
    });
  });

  it('says a cell is not pinned rather than offering a control that fails', async () => {
    // Llama at Q8_0 has a verdict but no pinned file. A download control there
    // would be a button whose only outcome is an error.
    render(<Llm />);

    const cell = await cellOf('Llama 3 8B', 'Q8_0');

    expect(cell).toHaveTextContent(/not pinned/i);
    expect(within(cell).queryByRole('button', { name: /^download /i })).toBeNull();
    expect(within(cell).queryByRole('button', { name: /^run/i })).toBeNull();
  });

  it('still offers download for a model the advisor says will not fit', async () => {
    // The calculator is an estimate; refusing on it would make osstat wrong in
    // a way the user cannot override. The verdict is stated beside the control
    // instead, so the choice is informed rather than removed.
    render(<Llm />);

    const cell = await cellOf('Qwen2.5 72B', 'Q4_K_M');

    expect(within(cell).getByRole('button', { name: /^download /i })).toBeEnabled();
    expect(
      within(cell).getByRole('button', { name: /larger than this machine/i })
    ).toBeInTheDocument();
  });

  it('re-reads the catalogue when a download finishes, so Run appears', async () => {
    render(<Llm />);
    await cellOf('Llama 3 8B', 'Q4_K_M');

    fetchModelCatalogue.mockResolvedValue(catalogue({ state: 'downloaded', path: LLAMA_PATH }));
    act(() => {
      done.fire({ key: { modelId: 'llama-3-8b', quantId: 'Q4_K_M' }, path: LLAMA_PATH });
    });

    expect(
      await screen.findByRole('button', { name: /^run llama 3 8b at q4_k_m$/i })
    ).toBeInTheDocument();
  });

  it('calls a checksum mismatch a verification failure and offers no retry', async () => {
    // The user-facing half of the security control. Retrying a hash that did
    // not match either wastes a multi-gigabyte download or invites someone to
    // keep trying until a tampered file slips through.
    render(<Llm />);
    await cellOf('Llama 3 8B', 'Q4_K_M');

    act(() => {
      failed.fire({
        key: { modelId: 'llama-3-8b', quantId: 'Q4_K_M' },
        message: 'Meta-Llama-3-8B-Instruct-Q4_K_M.gguf did not match its pinned checksum',
        retryable: false,
        verificationFailure: true,
        stopped: null,
      });
    });

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent(/verification failed/i);
    expect(alert).toHaveTextContent(/did not match its pinned checksum/i);

    const cell = await cellOf('Llama 3 8B', 'Q4_K_M');
    expect(within(cell).queryByRole('button', { name: /^retry /i })).toBeNull();
  });

  it('does not raise an alert for a pause the user asked for', async () => {
    // A deliberate pause is not a failure, and an alert saying so would train
    // people to ignore the one that matters.
    render(<Llm />);
    await downloading();

    act(() => {
      failed.fire({
        key: { modelId: 'llama-3-8b', quantId: 'Q4_K_M' },
        message: 'the download was paused; resuming continues from where it stopped',
        retryable: true,
        verificationFailure: false,
        stopped: 'pause',
      });
    });

    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('cancels the download the user started rather than every download', async () => {
    const user = userEvent.setup();
    render(<Llm />);

    const cell = await downloading();
    await user.click(within(cell).getByRole('button', { name: /^cancel /i }));

    expect(cancelModelDownload).toHaveBeenCalledTimes(1);
    expect(pauseModelDownload).not.toHaveBeenCalled();
  });
});

/**
 * What a cell offers while a model is loaded.
 *
 * The session now survives navigating away from the chat (ADR-013), so "a model
 * is already loaded" is the ordinary state of this page rather than a rare one —
 * and a Run control in that state invites the user to load what is loaded. At
 * best that is a click whose no-op cannot be told from a broken button; at worst
 * it is a close-and-reopen that throws away a live conversation to arrive back
 * where it started.
 *
 * Everything here is driven from `chat_status` through the shell, because that
 * is the claim being tested: the cell reports what Rust says is open, not what
 * this page last did. The harness below is the same wiring `App` does, so a
 * status the front end never asked for cannot reach a cell.
 */
describe('a cell whose model is loaded', () => {
  /** A second downloaded model, so "one cell only" has something to be wrong about. */
  const QWEN_PATH = 'D:\\models\\Qwen2.5-72B-Instruct-Q4_K_M.gguf';

  /** Both pinned models downloaded, which is when the two states are visible at once. */
  function bothDownloaded(): ModelCatalogueEntry[] {
    return catalogue({ state: 'downloaded', path: LLAMA_PATH }).map((entry) =>
      entry.key.modelId === 'qwen2.5-72b'
        ? { ...entry, state: 'downloaded' as const, path: QWEN_PATH }
        : entry
    );
  }

  /**
   * The page wired to the shell the way `App` wires it.
   *
   * The session lives above this page, is seeded from `chat_status`, and is
   * handed back down — so what a cell draws and what the chat page draws come
   * from one answer rather than two guesses.
   */
  function Wired({
    onModelOpened,
  }: {
    onModelOpened: (s: ModelSession) => void;
  }): React.JSX.Element {
    const [open, setOpen] = useState<ModelSession | null>(null);

    return <Llm openedModel={open} onSessionChange={setOpen} onModelOpened={onModelOpened} />;
  }

  it('reports the status instead of offering to run it again', async () => {
    chatStatus.mockResolvedValue(session());
    fetchModelCatalogue.mockResolvedValue(catalogue({ state: 'downloaded', path: LLAMA_PATH }));
    render(<Wired onModelOpened={() => undefined} />);

    const cell = await cellOf('Llama 3 8B', 'Q4_K_M');

    // Content, not presence: a control that said "Run" and behaved differently
    // would pass a test that only counted buttons.
    await waitFor(() => {
      expect(within(cell).getByRole('button', { name: /is loaded/i })).toHaveTextContent(
        /^Loaded$/
      );
    });
    expect(within(cell).queryByRole('button', { name: /^run /i })).toBeNull();
  });

  it('goes to the chat when the status is clicked', async () => {
    // Something sensible rather than nothing. There is nothing to open -- the
    // model is open -- so the only thing left to want is the conversation.
    const user = userEvent.setup();
    const onModelOpened = vi.fn();
    chatStatus.mockResolvedValue(session());
    fetchModelCatalogue.mockResolvedValue(catalogue({ state: 'downloaded', path: LLAMA_PATH }));
    render(<Wired onModelOpened={onModelOpened} />);

    const cell = await cellOf('Llama 3 8B', 'Q4_K_M');
    const status = await within(cell).findByRole('button', { name: /is loaded/i });
    await user.click(status);

    expect(onModelOpened).toHaveBeenCalledWith(session());
    // Nothing is re-opened. A status that quietly restarted the server would be
    // the defect this control was introduced to remove.
    expect(chatOpenModel).not.toHaveBeenCalled();
  });

  it('leaves every other downloaded model offering Run', async () => {
    // One session at a time, so exactly one cell may ever say so. Asserted
    // across the whole table rather than on the one cell, because the failure
    // this guards against is a second cell agreeing.
    chatStatus.mockResolvedValue(session());
    fetchModelCatalogue.mockResolvedValue(bothDownloaded());
    render(<Wired onModelOpened={() => undefined} />);

    const other = await cellOf('Qwen2.5 72B', 'Q4_K_M');
    await waitFor(() => {
      expect(within(other).getByRole('button', { name: /^run /i })).toHaveTextContent(/^Run$/);
    });
    expect(within(other).queryByRole('button', { name: /is loaded/i })).toBeNull();

    expect(screen.getAllByRole('button', { name: /is loaded/i })).toHaveLength(1);
  });

  it('goes back to Run when the model is unloaded', async () => {
    // The status is a reading, not a latch. Unloading happens on the chat page,
    // and this page has to stop claiming a model is running the moment it is
    // not -- otherwise the only way back to Run would be a restart.
    chatStatus.mockResolvedValue(session());
    fetchModelCatalogue.mockResolvedValue(catalogue({ state: 'downloaded', path: LLAMA_PATH }));

    function Unloadable(): React.JSX.Element {
      const [open, setOpen] = useState<ModelSession | null>(null);

      return (
        <>
          <button
            type="button"
            onClick={() => {
              setOpen(null);
            }}
          >
            Unload elsewhere
          </button>
          <Llm openedModel={open} onSessionChange={setOpen} />
        </>
      );
    }

    const user = userEvent.setup();
    render(<Unloadable />);

    const cell = await cellOf('Llama 3 8B', 'Q4_K_M');
    await within(cell).findByRole('button', { name: /is loaded/i });

    await user.click(screen.getByRole('button', { name: 'Unload elsewhere' }));

    await waitFor(() => {
      expect(within(cell).getByRole('button', { name: /^run /i })).toHaveTextContent(/^Run$/);
    });
    expect(within(cell).queryByRole('button', { name: /is loaded/i })).toBeNull();
  });

  it('shows no status at all when Rust says nothing is open', async () => {
    // The direction that costs the user something. A status drawn from a stale
    // front-end flag would hide the Run control for a model nothing is holding.
    chatStatus.mockResolvedValue(null);
    fetchModelCatalogue.mockResolvedValue(bothDownloaded());
    render(<Wired onModelOpened={() => undefined} />);

    const cell = await cellOf('Llama 3 8B', 'Q4_K_M');

    expect(within(cell).getByRole('button', { name: /^run /i })).toHaveTextContent(/^Run$/);
    expect(screen.queryByRole('button', { name: /is loaded/i })).toBeNull();
  });
});
