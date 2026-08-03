import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { LlmAdvice } from '../bindings/LlmAdvice';
import type { ModelRegistry } from '../bindings/ModelRegistry';

const fetchModelRegistry = vi.fn<() => Promise<ModelRegistry>>();
const fetchLlmAdvice = vi.fn<(contextLength: number) => Promise<LlmAdvice | null>>();
const onGpusReady = vi.fn();

// The acquisition controls are covered in `Llm.download.test.tsx`; here they
// only need to exist, and to answer with an empty catalogue so every cell says
// "not pinned" rather than crashing on a missing mock.
vi.mock('../lib/ipc', () => ({
  fetchModelRegistry: (): Promise<ModelRegistry> => fetchModelRegistry(),
  fetchLlmAdvice: (contextLength: number): Promise<LlmAdvice | null> =>
    fetchLlmAdvice(contextLength),
  onGpusReady: (handler: () => void): Promise<() => void> => onGpusReady(handler),
  fetchModelCatalogue: (): Promise<[]> => Promise.resolve([]),
  downloadModel: (): Promise<void> => Promise.resolve(),
  cancelModelDownload: (): Promise<void> => Promise.resolve(),
  chatOpenModel: (): Promise<never> => Promise.reject(new Error('not used here')),
  chatStatus: (): Promise<null> => Promise.resolve(null),
  onModelProgress: (): Promise<() => void> => Promise.resolve(() => undefined),
  onModelDone: (): Promise<() => void> => Promise.resolve(() => undefined),
  onModelFailed: (): Promise<() => void> => Promise.resolve(() => undefined),
}));

const { Llm } = await import('./Llm');

function registry(): ModelRegistry {
  return {
    version: 1,
    quantLevels: [
      { id: 'Q4_K_M', label: 'Q4_K_M', bitsPerWeight: 4.85, description: 'A good default.' },
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
    ],
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  onGpusReady.mockResolvedValue(() => {});
  fetchModelRegistry.mockResolvedValue(registry());
  fetchLlmAdvice.mockResolvedValue(advice());
});

describe('Llm', () => {
  it('waits for the probe rather than reporting no GPU on a machine that has one', async () => {
    // `null` is the probe still running. Rendering a CPU-only matrix here
    // would be a confident wrong answer, which is the one thing ADR-008 asks
    // this feature never to give.
    fetchLlmAdvice.mockResolvedValue(null);
    render(<Llm />);

    // Matched by text rather than by role: the initial "reading the
    // registry" state is also a `status`, and is already on screen when the
    // query runs, so a role query would resolve against the wrong one.
    expect(await screen.findByText(/looking for a gpu/i)).toBeInTheDocument();
    expect(screen.queryByRole('table')).not.toBeInTheDocument();
  });

  it('shows the arithmetic behind a verdict when its cell is picked', async () => {
    const user = userEvent.setup();
    render(<Llm />);

    const cell = await screen.findByRole('button', {
      name: /llama 3 8b at q4_k_m: fits entirely in vram/i,
    });
    await user.click(cell);

    const drawer = await screen.findByRole('region', {
      name: /how llama 3 8b at q4_k_m was worked out/i,
    });

    // The three terms plus the total: the drawer's whole reason to exist is
    // that these are visible rather than folded into one number.
    expect(drawer).toHaveTextContent(/weights/i);
    expect(drawer).toHaveTextContent(/runtime overhead/i);
    expect(drawer).toHaveTextContent(/kv cache/i);
    expect(drawer).toHaveTextContent(/total required/i);
    // And that the speed tier is not passed off as a measurement.
    expect(drawer).toHaveTextContent(/classification, not a measurement/i);
  });

  it('re-prices the matrix when the context length changes', async () => {
    const user = userEvent.setup();
    render(<Llm />);
    await screen.findByRole('table');

    await user.click(screen.getByRole('button', { name: '8K' }));

    await waitFor(() => {
      expect(fetchLlmAdvice).toHaveBeenCalledWith(8192);
    });
  });

  it('flags a row priced past the model’s own maximum context', async () => {
    const user = userEvent.setup();
    fetchLlmAdvice.mockResolvedValue({ ...advice(), contextLength: 32768 });
    render(<Llm />);
    await screen.findByRole('table');

    // The fixture model tops out at 8192, so pricing it at 32768 describes a
    // configuration it cannot be loaded at — correct arithmetic, impossible
    // setup, and the table has to say which.
    expect(screen.getByText(/past 8K/i)).toBeInTheDocument();

    await user.click(
      screen.getByRole('button', { name: /llama 3 8b at q4_k_m: fits entirely in vram/i })
    );
    expect(await screen.findByText(/natively accepts 8K tokens/i)).toBeInTheDocument();
  });

  it('says when a GPU is present but its VRAM is unreported', async () => {
    fetchLlmAdvice.mockResolvedValue({
      ...advice(),
      gpu: { present: true, vramBytes: null },
    });
    render(<Llm />);

    expect(await screen.findByText(/does not report vram/i)).toBeInTheDocument();
    expect(screen.getByText('not reported')).toBeInTheDocument();
  });
});
