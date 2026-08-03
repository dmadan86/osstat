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
 * The second idea, **corrected**: a searched result shows its file size until
 * somebody asks for the fit, and then it shows a real verdict. This file used to
 * assert the opposite — that a searched result must never show one — on the
 * belief that the architecture is only available after downloading. That belief
 * was wrong: a GGUF header sits at the *start* of the file, so Rust fetches it
 * with a `Range` request and prices it with the same `plan_launch` a downloaded
 * model gets. ADR-008's rule stands untouched; what changed is that the number
 * is now measured rather than unavailable, so declining to show it is no longer
 * the honest answer, it is just a missing feature.
 *
 * What must still never appear is a verdict for a header that was **not** read.
 * A row nobody expanded, and a row whose header could not be fetched, both show
 * the size alone — see the two tests below that hold that line.
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
import type { SearchedFit } from '../bindings/SearchedFit';
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
  priceSearchedModel,
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
  priceSearchedModel: vi.fn(),
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
  priceSearchedModel,
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

/**
 * A verdict of the shape `models_price_searched` returns.
 *
 * Every field is one `plan_launch` produced over a header read off the front of
 * the file — the same call and the same struct a downloaded model is opened
 * with. Nothing here is derived from `sizeBytes`, which is the whole point.
 */
function fit(overrides: Partial<SearchedFit> = {}): SearchedFit {
  return {
    gpuLayers: 40,
    blockCount: 40,
    contextLength: 8192,
    fits: true,
    headDimDerived: false,
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

  it('shows no verdict for a searched result until its header is asked for', async () => {
    // The replacement for a test that asserted a searched result must NEVER
    // show a verdict. The line that actually matters is narrower and still
    // holds: no verdict for a header nobody has read. An unexpanded row has not
    // cost a request, so it has nothing to say beyond its size.
    searchModels.mockResolvedValue([result()]);
    render(<Llm />);
    await screen.findByRole('table');

    await search();

    const section = resultsSection();
    await within(section).findByRole('group', { name: /Mistral-Nemo/ });

    // The vocabulary a verdict is made of. None of it can be justified before
    // the header has been read, and none of it appears.
    for (const verdict of [
      /fits entirely in vram/i,
      /layers on gpu/i,
      /rest on the cpu/i,
      /\bcontext\b/i,
    ]) {
      expect(within(section).queryByText(verdict)).toBeNull();
    }

    expect(section).toHaveTextContent('8.13 GB');
    // Nothing was fetched, which is the property that keeps a page of results
    // from becoming a page of requests.
    expect(priceSearchedModel).not.toHaveBeenCalled();
  });

  it('prices a searched result from its header when the row is expanded', async () => {
    // The correction this file exists to record. The header is at the front of
    // the file, so the verdict is the real one -- the same `plan_launch` a
    // downloaded model is opened with, over the same bytes.
    searchModels.mockResolvedValue([result()]);
    priceSearchedModel.mockResolvedValue(fit());
    render(<Llm />);
    await screen.findByRole('table');

    await search();

    const user = userEvent.setup();
    await user.click(
      await within(resultsSection()).findByRole('button', {
        name: /Check fit for Mistral-Nemo-Instruct-Q5_K_M\.gguf/i,
      })
    );

    const row = await within(resultsSection()).findByRole('group', { name: /Mistral-Nemo/ });
    await waitFor(() => {
      expect(row).toHaveTextContent(/40 of 40 layers|fits entirely in vram/i);
    });

    // Priced from the actual result, unaltered -- Rust re-validates it, so a
    // reshaped one would be refused and read as a broken button.
    expect(priceSearchedModel).toHaveBeenCalledWith(result());
  });

  it('says a searched result is checking while its header is being read', async () => {
    // A row that showed nothing between the click and the answer would read as
    // a control that did not work, over a request that can take seconds.
    searchModels.mockResolvedValue([result()]);
    priceSearchedModel.mockReturnValue(new Promise(() => {}));
    render(<Llm />);
    await screen.findByRole('table');

    await search();

    const user = userEvent.setup();
    await user.click(
      await within(resultsSection()).findByRole('button', { name: /Check fit for Mistral-Nemo/i })
    );

    expect(await within(resultsSection()).findByText(/checking the fit/i)).toBeInTheDocument();
  });

  it('shows a partial offload as the fraction it is', async () => {
    // `fits: false` is a warning and never a refusal -- the arithmetic is an
    // estimate, and refusing on it would make osstat wrong in a way the user
    // cannot override. The row has to say so rather than just failing.
    searchModels.mockResolvedValue([result()]);
    priceSearchedModel.mockResolvedValue(fit({ gpuLayers: 12, fits: false }));
    render(<Llm />);
    await screen.findByRole('table');

    await search();

    const user = userEvent.setup();
    await user.click(
      await within(resultsSection()).findByRole('button', { name: /Check fit for Mistral-Nemo/i })
    );

    const row = await within(resultsSection()).findByRole('group', { name: /Mistral-Nemo/ });
    await waitFor(() => {
      expect(row).toHaveTextContent(/12 of 40 layers on GPU/i);
    });
    expect(row).toHaveTextContent(/rest on the CPU/i);
  });

  it('falls back to the size alone when the header cannot be read', async () => {
    // A server that ignores `Range` sends the whole file, so Rust stops at its
    // ceiling and reports the result unpriced. Guessing a verdict from the size
    // instead is the one failure ADR-008 names -- so the row says what it does
    // not know, which is the honest answer and still leaves Download offered.
    searchModels.mockResolvedValue([result()]);
    priceSearchedModel.mockRejectedValue(
      new Error('the header did not appear in the first 67108864 bytes of the file')
    );
    render(<Llm />);
    await screen.findByRole('table');

    await search();

    const user = userEvent.setup();
    await user.click(
      await within(resultsSection()).findByRole('button', { name: /Check fit for Mistral-Nemo/i })
    );

    const row = await within(resultsSection()).findByRole('group', { name: /Mistral-Nemo/ });
    await waitFor(() => {
      expect(row).toHaveTextContent(/header could not be read/i);
    });
    expect(row).toHaveTextContent(/first 67108864 bytes/i);

    // The size stays, and no verdict was invented to fill the gap.
    expect(row).toHaveTextContent('8.13 GB');
    expect(within(row).queryByText(/layers on gpu/i)).toBeNull();
    expect(within(row).getByRole('button', { name: /Download Mistral-Nemo/i })).toBeInTheDocument();
  });

  it('reads a header once however often a row is opened and shut', async () => {
    // One request per row, not one per click. This is a `Range` request against
    // a file that can be thirty gigabytes; a toggle that re-fetched would make
    // an idle fidget expensive.
    searchModels.mockResolvedValue([result()]);
    priceSearchedModel.mockResolvedValue(fit());
    render(<Llm />);
    await screen.findByRole('table');

    await search();

    const user = userEvent.setup();
    const toggle = await within(resultsSection()).findByRole('button', {
      name: /Check fit for Mistral-Nemo/i,
    });
    await user.click(toggle);
    await user.click(toggle);
    await user.click(toggle);

    await waitFor(() => {
      expect(priceSearchedModel).toHaveBeenCalledTimes(1);
    });
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
