/**
 * Searching Hugging Face from the LLM tab.
 *
 * The governing idea, and what every test here defends: osstat now has **two
 * verification tiers and they must stay visibly different**. A pinned model is
 * checked against a hash reviewed in a pull request against this repository. A
 * searched one is checked against the hash Hugging Face reports beside the
 * file, which catches a corrupted transfer and cannot catch a replaced upload.
 * A search result that downloaded exactly like a pinned one, with no visible
 * difference, would quietly retire a guarantee SECURITY.md still makes.
 *
 * The second idea: a searched result shows its **file size and nothing else**.
 * No fit verdict, no speed tier, no layer estimate. The advisor prices a model
 * from architecture it has not read yet, and a number derived from file size
 * and quantization bits would look like the calculator's output while being a
 * guess — the failure ADR-008 names as the worst thing this feature could do.
 *
 * Assertions are on **content**, as in `Llm.download.test.tsx`: a test that
 * only checked "a section exists" would pass with the label deleted, which is
 * the one defect this file exists to catch. `../lib/ipc` is mocked, so nothing
 * here makes a request.
 */

import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { LlmAdvice } from '../bindings/LlmAdvice';
import type { ModelCatalogueEntry } from '../bindings/ModelCatalogueEntry';
import type { ModelRegistry } from '../bindings/ModelRegistry';
import type { SearchResult } from '../bindings/SearchResult';

const {
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
  searchModels,
} = vi.hoisted(() => ({
  cancelModelDownload: vi.fn(),
  chatOpenModel: vi.fn(),
  downloadModel: vi.fn(),
  downloadSearchedModel: vi.fn(),
  fetchLlmAdvice: vi.fn(),
  fetchModelCatalogue: vi.fn(),
  fetchModelRegistry: vi.fn(),
  onGpusReady: vi.fn(),
  onModelDone: vi.fn(),
  onModelFailed: vi.fn(),
  onModelProgress: vi.fn(),
  pauseModelDownload: vi.fn(),
  searchModels: vi.fn(),
}));

vi.mock('../lib/ipc', () => ({
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
  searchModels,
}));

const { Llm } = await import('./Llm');

/** A subscription that never fires, for the events these tests do not drive. */
function inert(): Promise<() => void> {
  return Promise.resolve(() => {
    // Nothing was subscribed, so there is nothing to undo.
  });
}

/** The pinned registry: one model, two quantizations. */
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
    ],
  };
}

/** One breakdown, reused: these tests are about the label, not the arithmetic. */
function breakdown() {
  return {
    contextLength: 4096,
    quantizedWeightBytes: 4_850_000_000,
    overheadBytes: 485_000_000,
    kvCacheBytes: 536_870_912,
    totalRequiredBytes: 5_871_870_912,
    availableVramBytes: 25_769_803_776,
    availableSystemMemoryBytes: 68_719_476_736,
  };
}

function advice(): LlmAdvice {
  return {
    contextLength: 4096,
    systemMemoryBytes: 68_719_476_736,
    gpu: { present: true, vramBytes: 25_769_803_776 },
    results: [
      {
        modelId: 'llama-3-8b',
        quantId: 'Q4_K_M',
        verdict: { kind: 'fitsOnGpu', gpuLayers: 32, cpuLayers: 0, tier: 'fast' },
        breakdown: breakdown(),
      },
      {
        modelId: 'llama-3-8b',
        quantId: 'Q8_0',
        verdict: { kind: 'wontFit', gpuLayers: 0, cpuLayers: 32, tier: 'unusable' },
        breakdown: breakdown(),
      },
    ],
  } as unknown as LlmAdvice;
}

/** The pinned catalogue: Llama at Q4_K_M, pinned and not yet downloaded. */
function catalogue(extra: ModelCatalogueEntry[] = []): ModelCatalogueEntry[] {
  return [
    {
      key: { modelId: 'llama-3-8b', quantId: 'Q4_K_M' },
      state: 'downloadable',
      publisher: 'bartowski',
      repo: 'bartowski/Meta-Llama-3-8B-Instruct-GGUF',
      file: 'Meta-Llama-3-8B-Instruct-Q4_K_M.gguf',
      sizeBytes: 4_920_734_688,
      path: null,
      provenance: 'pinned',
    },
    ...extra,
  ];
}

/** A result of the shape `models_search` returns. */
function result(overrides: Partial<SearchResult> = {}): SearchResult {
  return {
    repo: 'TheOtherOne/Mistral-Nemo-GGUF',
    publisher: 'TheOtherOne',
    file: 'Mistral-Nemo-Instruct-Q5_K_M.gguf',
    sizeBytes: 8_727_493_120,
    sha256: 'c'.repeat(64),
    quantHint: 'Q5_K_M',
    ...overrides,
  };
}

/** Types a term into the search box and submits it. */
async function search(term = 'mistral'): Promise<void> {
  const user = userEvent.setup();
  await user.type(screen.getByRole('searchbox', { name: /search/i }), term);
  await user.click(screen.getByRole('button', { name: /^search$/i }));
}

/** The section searched results live in, which is not the pinned matrix. */
function resultsSection(): HTMLElement {
  return screen.getByRole('region', { name: /found on hugging face/i });
}

beforeEach(() => {
  vi.clearAllMocks();
  fetchModelRegistry.mockResolvedValue(registry());
  fetchLlmAdvice.mockResolvedValue(advice());
  fetchModelCatalogue.mockResolvedValue(catalogue());
  searchModels.mockResolvedValue([]);
  onGpusReady.mockImplementation(inert);
  onModelProgress.mockImplementation(inert);
  onModelDone.mockImplementation(inert);
  onModelFailed.mockImplementation(inert);
  downloadSearchedModel.mockResolvedValue(undefined);
});

describe('Llm › searching Hugging Face', () => {
  it('shows a searched result with its size and publisher', async () => {
    searchModels.mockResolvedValue([result()]);
    render(<Llm />);
    await screen.findByRole('table');

    await search();

    const row = await within(resultsSection()).findByRole('group', {
      name: /Mistral-Nemo-Instruct-Q5_K_M\.gguf/,
    });
    expect(row).toHaveTextContent('8.13 GB');
    expect(row).toHaveTextContent('TheOtherOne');
  });

  it('does not show a fit verdict for a searched result', async () => {
    // The advisor cannot price a model whose architecture it has not read.
    // A verdict here would be a guess wearing the calculator's clothes.
    searchModels.mockResolvedValue([result()]);
    render(<Llm />);
    await screen.findByRole('table');

    await search();

    const section = resultsSection();
    await within(section).findByRole('group', { name: /Mistral-Nemo/ });

    // The pinned matrix's whole vocabulary for pricing a model: the four
    // verdict labels, the four short badges, the three speed tiers and the
    // terms of the arithmetic. None of it is anything this page could justify
    // for a file whose header it has not read.
    for (const verdict of [
      /fits entirely in vram/i,
      /fits across vram/i,
      /fits in system memory/i,
      /larger than this machine/i,
      /\bvram\b/i,
      /\bgpu\b/i,
      /\bcpu\b/i,
      /offload/i,
      /\bfast\b/i,
      /\bmoderate\b/i,
      /\bslow\b/i,
      /layer/i,
      /kv cache/i,
    ]) {
      expect(within(section).queryByText(verdict)).toBeNull();
    }

    // And the honest positive: a size, and no second figure beside it.
    expect(section).toHaveTextContent('8.13 GB');
    expect(within(section).getByText(/size is all osstat can say/i)).toBeInTheDocument();
  });

  it('marks a searched model as unreviewed, distinctly from a pinned one', async () => {
    // The label IS the feature. Without it this quietly retires a guarantee
    // SECURITY.md still makes.
    searchModels.mockResolvedValue([result()]);
    render(<Llm />);
    const matrix = await screen.findByRole('table');

    await search();

    const row = await within(resultsSection()).findByRole('group', { name: /Mistral-Nemo/ });
    expect(row).toHaveTextContent(/not reviewed/i);
    expect(row).toHaveTextContent(/hugging face/i);

    // The distinction, not just the presence of a word: the curated matrix must
    // not carry the same label, or the two tiers read identically.
    expect(within(matrix).queryByText(/not reviewed/i)).toBeNull();
    expect(matrix).toHaveTextContent(/bartowski/);
  });

  it('says when a search returns nothing, rather than showing an empty area', async () => {
    searchModels.mockResolvedValue([]);
    render(<Llm />);
    await screen.findByRole('table');

    await search('something nobody uploaded');

    expect(await screen.findByText(/nothing on hugging face matched that/i)).toBeInTheDocument();
  });

  it('still shows the seven pinned models with their verdicts', async () => {
    // Search must not displace the curated matrix.
    searchModels.mockResolvedValue([result()]);
    render(<Llm />);
    const matrix = await screen.findByRole('table');

    await search();

    await within(resultsSection()).findByRole('group', { name: /Mistral-Nemo/ });

    expect(within(matrix).getByText('Llama 3 8B')).toBeInTheDocument();
    expect(
      within(matrix).getByRole('button', {
        name: /Llama 3 8B at Q4_K_M: Fits entirely in VRAM/i,
      })
    ).toBeInTheDocument();
    expect(
      within(matrix).getByRole('button', {
        name: /Download Llama 3 8B at Q4_K_M, 4\.58 GB, via bartowski/i,
      })
    ).toBeInTheDocument();
  });

  it('hands the result back to Rust unaltered when it is downloaded', async () => {
    // Rust re-validates what it is given, so the front end must not reshape it:
    // an edited result is refused, which would read as a broken button.
    const found = result();
    searchModels.mockResolvedValue([found]);
    render(<Llm />);
    await screen.findByRole('table');

    await search();

    const user = userEvent.setup();
    await user.click(
      await within(resultsSection()).findByRole('button', {
        name: /Download Mistral-Nemo-Instruct-Q5_K_M\.gguf/i,
      })
    );

    await waitFor(() => {
      expect(downloadSearchedModel).toHaveBeenCalledWith(found);
    });
  });

  it('reports a search that could not be made, rather than showing nothing found', async () => {
    // "Hugging Face did not answer" and "nothing matched" are different things
    // to tell someone, and only one of them is worth trying again.
    searchModels.mockRejectedValue(new Error('could not fetch the model list'));
    render(<Llm />);
    await screen.findByRole('table');

    await search();

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent(/could not fetch the model list/i);
    expect(screen.queryByText(/nothing on hugging face matched that/i)).toBeNull();
  });

  it('keeps a downloaded searched model listed with its label after the search is cleared', async () => {
    // The catalogue is what survives a restart. A model that only carried its
    // label while the search box still held the term would lose the tier it was
    // fetched under the moment the page was reloaded.
    fetchModelCatalogue.mockResolvedValue(
      catalogue([
        {
          key: {
            modelId: 'TheOtherOne/Mistral-Nemo-GGUF',
            quantId: 'Mistral-Nemo-Instruct-Q5_K_M.gguf',
          },
          state: 'downloaded',
          publisher: 'TheOtherOne',
          repo: 'TheOtherOne/Mistral-Nemo-GGUF',
          file: 'Mistral-Nemo-Instruct-Q5_K_M.gguf',
          sizeBytes: 8_727_493_120,
          path: 'D:\\models\\Mistral-Nemo-Instruct-Q5_K_M.gguf',
          provenance: 'searched',
        },
      ])
    );
    render(<Llm />);
    await screen.findByRole('table');

    const row = await screen.findByRole('group', {
      name: /Mistral-Nemo-Instruct-Q5_K_M\.gguf/,
    });

    expect(row).toHaveTextContent(/not reviewed/i);
    expect(within(row).getByRole('button', { name: /^Run /i })).toBeInTheDocument();
  });
});
