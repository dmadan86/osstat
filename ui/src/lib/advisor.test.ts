import { describe, expect, it } from 'vitest';

import type { FitResult } from '../bindings/FitResult';
import type { GpuBudget } from '../bindings/GpuBudget';
import type { ModelEntry } from '../bindings/ModelEntry';
import type { VerdictKind } from '../bindings/VerdictKind';
import {
  budgetCaveat,
  cellKey,
  countByVerdict,
  exceedsNativeContext,
  formatTokens,
  indexResults,
  VERDICTS,
} from './advisor';

function model(maxContextLength: number, id = 'test'): ModelEntry {
  return {
    id,
    name: 'Test',
    family: 'Test',
    parametersBillion: 7,
    architecture: {
      numLayers: 32,
      hiddenSize: 4096,
      numAttentionHeads: 32,
      numKvHeads: 8,
      headDim: 128,
      maxContextLength,
    },
    sourceNote: 'fixture',
  };
}

function result(modelId: string, quantId: string, kind: VerdictKind): FitResult {
  return {
    modelId,
    quantId,
    verdict: { kind, gpuLayers: 32, cpuLayers: 0, tier: 'fast' },
    breakdown: {
      quantizedWeightBytes: 1,
      overheadBytes: 1,
      kvCacheBytes: 1,
      totalRequiredBytes: 3,
      availableVramBytes: 4,
      availableSystemMemoryBytes: 5,
      contextLength: 4096,
    },
  };
}

describe('cellKey', () => {
  it('does not let two different pairs collide on one key', () => {
    // Both ids may contain dashes, so a dash separator would make
    // ("a-b", "c") and ("a", "b-c") the same key.
    expect(cellKey('a-b', 'c')).not.toBe(cellKey('a', 'b-c'));
  });
});

describe('indexResults', () => {
  it('finds every cell it was given', () => {
    const index = indexResults([
      result('llama', 'Q4', 'fitsOnGpu'),
      result('llama', 'Q8', 'wontFit'),
    ]);

    expect(index.get(cellKey('llama', 'Q4'))?.verdict.kind).toBe('fitsOnGpu');
    expect(index.get(cellKey('llama', 'Q8'))?.verdict.kind).toBe('wontFit');
    expect(index.get(cellKey('llama', 'Q2'))).toBeUndefined();
  });
});

describe('exceedsNativeContext', () => {
  it('is false at exactly the model’s maximum', () => {
    expect(exceedsNativeContext(model(8192), 8192)).toBe(false);
  });

  it('is true one token past it', () => {
    expect(exceedsNativeContext(model(8192), 8193)).toBe(true);
  });
});

describe('budgetCaveat', () => {
  it('says so when no GPU was found', () => {
    const gpu: GpuBudget = { present: false, vramBytes: null };
    expect(budgetCaveat(gpu)).toMatch(/no gpu/i);
  });

  it('distinguishes a GPU with no reported VRAM from no GPU at all', () => {
    // The distinction ADR-008 cares about: a missing number the user can see
    // is missing, rather than a confident wrong one.
    const unknown = budgetCaveat({ present: true, vramBytes: null });
    const absent = budgetCaveat({ present: false, vramBytes: null });

    expect(unknown).not.toBeNull();
    expect(unknown).not.toBe(absent);
    expect(unknown).toMatch(/does not report vram/i);
  });

  it('has nothing to caveat when VRAM is a real figure', () => {
    expect(budgetCaveat({ present: true, vramBytes: 8_000_000_000 })).toBeNull();
  });
});

describe('countByVerdict', () => {
  it('counts each kind and leaves the others at zero', () => {
    const counts = countByVerdict([
      result('a', 'Q4', 'fitsOnGpu'),
      result('b', 'Q4', 'fitsOnGpu'),
      result('c', 'Q4', 'wontFit'),
    ]);

    expect(counts.fitsOnGpu).toBe(2);
    expect(counts.wontFit).toBe(1);
    expect(counts.fitsWithCpuOffload).toBe(0);
    expect(counts.fitsOnCpuOnly).toBe(0);
  });
});

describe('formatTokens', () => {
  it('uses K only where it divides exactly', () => {
    expect(formatTokens(4096)).toBe('4K');
    expect(formatTokens(131072)).toBe('128K');
    expect(formatTokens(1000)).toBe('1000');
    expect(formatTokens(5000)).toBe('5000');
  });
});

describe('VERDICTS', () => {
  it('words every verdict the backend can produce', () => {
    // A missing entry would render `undefined` into a cell rather than fail,
    // so this is asserted rather than left to the type checker alone.
    const kinds: VerdictKind[] = ['fitsOnGpu', 'fitsWithCpuOffload', 'fitsOnCpuOnly', 'wontFit'];
    for (const kind of kinds) {
      expect(VERDICTS[kind].short).toBeTruthy();
      expect(VERDICTS[kind].label).toBeTruthy();
    }
  });
});
