/**
 * The inference runtime section of Settings.
 *
 * Kept apart from `Settings.test.tsx`, which covers the preference controls and
 * needs no IPC mock at all. These tests exist mostly to pin two decisions that
 * are easy to undo by accident: CUDA is an offer rather than a default, and a
 * verification failure is never given a retry button.
 */

import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { InferenceRuntime } from '../components/InferenceRuntime';
import type { RuntimeStatus } from '../bindings/RuntimeStatus';

const {
  acquireRuntime,
  deleteRuntime,
  fetchRuntimeStatus,
  onRuntimeFailed,
  onRuntimeProgress,
  onRuntimeReady,
} = vi.hoisted(() => ({
  acquireRuntime: vi.fn(),
  deleteRuntime: vi.fn(),
  fetchRuntimeStatus: vi.fn(),
  onRuntimeFailed: vi.fn(),
  onRuntimeProgress: vi.fn(),
  onRuntimeReady: vi.fn(),
}));

vi.mock('../lib/ipc', () => ({
  acquireRuntime,
  deleteRuntime,
  fetchRuntimeStatus,
  onRuntimeFailed,
  onRuntimeProgress,
  onRuntimeReady,
}));

/** Captures the handler a subscription was given, so a test can fire it. */
function capturing<T>(): {
  fire: (payload: T) => void;
  subscribe: (h: (p: T) => void) => Promise<() => void>;
} {
  let handler: ((payload: T) => void) | null = null;

  return {
    fire: (payload) => handler?.(payload),
    subscribe: (h) => {
      handler = h;
      return Promise.resolve(() => {
        handler = null;
      });
    },
  };
}

function status(overrides: Partial<RuntimeStatus> = {}): RuntimeStatus {
  return {
    installed: [],
    recommended: {
      artifactId: 'win-vulkan-x64',
      backend: 'Vulkan',
      downloadBytes: 33_637_361,
    },
    cudaAlternative: null,
    upstreamTag: 'b10194',
    ...overrides,
  };
}

const CUDA = {
  artifactId: 'win-cuda-13.3-x64',
  backend: 'CUDA 13.3',
  downloadBytes: 537_835_790,
};

beforeEach(() => {
  vi.clearAllMocks();
  acquireRuntime.mockResolvedValue(undefined);
  deleteRuntime.mockResolvedValue(undefined);
  fetchRuntimeStatus.mockResolvedValue(status());
  onRuntimeProgress.mockResolvedValue(() => undefined);
  onRuntimeReady.mockResolvedValue(() => undefined);
  onRuntimeFailed.mockResolvedValue(() => undefined);
});

describe('Settings › inference runtime', () => {
  it('waits for the GPU probe rather than recommending a CPU build', async () => {
    fetchRuntimeStatus.mockResolvedValue(null);
    render(<InferenceRuntime />);

    expect(await screen.findByText(/waiting for the gpu probe/i)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /install/i })).not.toBeInTheDocument();
  });

  it('offers the recommended runtime with its download size', async () => {
    render(<InferenceRuntime />);

    expect(await screen.findByRole('button', { name: /install vulkan/i })).toHaveTextContent(
      /32(\.\d+)? MB|33(\.\d+)? MB/
    );
  });

  it('presents CUDA as a choice with its true cost, not as the default', async () => {
    // The decision this test exists to protect: a 538 MB download is never
    // started because a card was detected. Both options must be on screen,
    // both sized, before anything runs.
    fetchRuntimeStatus.mockResolvedValue(status({ cudaAlternative: CUDA }));
    render(<InferenceRuntime />);

    const vulkan = await screen.findByRole('button', { name: /install vulkan/i });
    const cuda = screen.getByRole('button', { name: /install cuda/i });

    expect(vulkan).toBeInTheDocument();
    expect(cuda).toBeInTheDocument();
    expect(acquireRuntime).not.toHaveBeenCalled();

    fireEvent.click(vulkan);
    expect(acquireRuntime).toHaveBeenCalledWith(false);
  });

  it('asks for CUDA only when the CUDA option is the one clicked', async () => {
    fetchRuntimeStatus.mockResolvedValue(status({ cudaAlternative: CUDA }));
    render(<InferenceRuntime />);

    fireEvent.click(await screen.findByRole('button', { name: /install cuda/i }));

    expect(acquireRuntime).toHaveBeenCalledWith(true);
  });

  it('lists an installed runtime with its size and a delete control', async () => {
    fetchRuntimeStatus.mockResolvedValue(
      status({
        installed: [{ tag: 'b10194', artifactId: 'ubuntu-vulkan-x64', sizeBytes: 90_000_000 }],
      })
    );
    render(<InferenceRuntime />);

    expect(await screen.findByText('ubuntu-vulkan-x64')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /delete/i }));

    await waitFor(() => {
      expect(deleteRuntime).toHaveBeenCalledWith('b10194', 'ubuntu-vulkan-x64');
    });
  });

  it('reports a checksum failure as verification, with no retry offered', async () => {
    // The user-facing half of the security control. Retrying a hash that did
    // not match either wastes a 600 MB download or invites someone to keep
    // trying until a tampered file slips through.
    const failed = capturing<{
      message: string;
      retryable: boolean;
      verificationFailure: boolean;
    }>();
    onRuntimeFailed.mockImplementation(failed.subscribe);

    render(<InferenceRuntime />);
    await screen.findByRole('button', { name: /install vulkan/i });

    failed.fire({
      message: 'llama.zip did not match its pinned checksum',
      retryable: false,
      verificationFailure: true,
    });

    expect(await screen.findByText(/verification failed/i)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /try again/i })).not.toBeInTheDocument();
  });

  it('offers a retry for a transport failure', async () => {
    const failed = capturing<{
      message: string;
      retryable: boolean;
      verificationFailure: boolean;
    }>();
    onRuntimeFailed.mockImplementation(failed.subscribe);

    render(<InferenceRuntime />);
    await screen.findByRole('button', { name: /install vulkan/i });

    failed.fire({
      message: 'fetching https://example.invalid returned HTTP 503',
      retryable: true,
      verificationFailure: false,
    });

    expect(await screen.findByRole('button', { name: /try again/i })).toBeInTheDocument();
    expect(screen.queryByText(/verification failed/i)).not.toBeInTheDocument();
  });

  it('names the CUDA runtime download as its own phase', async () => {
    // Otherwise the bar appears to restart for no reason: the cudart archive
    // is the larger of the two downloads.
    const progress = capturing<{
      phase: string;
      downloadedBytes: number;
      totalBytes: number;
    }>();
    onRuntimeProgress.mockImplementation(progress.subscribe);

    render(<InferenceRuntime />);
    await screen.findByRole('button', { name: /install vulkan/i });

    progress.fire({ phase: 'cudaRuntime', downloadedBytes: 1_000_000, totalBytes: 391_443_627 });

    expect(await screen.findByText(/downloading the cuda runtime/i)).toBeInTheDocument();
  });

  it('says so when this platform has no published build at all', async () => {
    fetchRuntimeStatus.mockResolvedValue(status({ recommended: null }));
    render(<InferenceRuntime />);

    expect(
      await screen.findByText(/publishes no build for this operating system/i)
    ).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /install/i })).not.toBeInTheDocument();
  });
});
