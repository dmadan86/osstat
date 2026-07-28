/**
 * Subscriptions to the live data the backend pushes.
 *
 * Each hook seeds itself from a command and then follows the event stream, so a
 * chart paints filled on its first render instead of growing in from an empty
 * axis. That history lives in Rust — see `src-tauri/src/sampler.rs` — which is
 * also why a reload or a navigation change does not reset it.
 */

import { useEffect, useMemo, useRef, useState } from 'react';

import type { GpuDevice } from '../bindings/GpuDevice';
import type { MetricsSample } from '../bindings/MetricsSample';
import type { ProcessRecord } from '../bindings/ProcessRecord';
import type { SystemDescription } from '../bindings/SystemDescription';
import {
  fetchGpuDevices,
  fetchMetricsHistory,
  fetchProcessList,
  fetchSystemDescription,
  onGpusReady,
  onMetricsTick,
  onProcessesTick,
  onTrayHidden,
} from './ipc';
import { applyDiff, buildTree, keyOf, type ProcessTree } from './processTree';

/** What a load can be doing. */
export type LoadState<T> =
  { status: 'loading' } | { status: 'ready'; value: T } | { status: 'error'; message: string };

/** Renders an unknown thrown value as a message. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Loads the machine's description once.
 *
 * @returns The description, or the reason it could not be read.
 */
export function useSystemDescription(): LoadState<SystemDescription> {
  const [state, setState] = useState<LoadState<SystemDescription>>({ status: 'loading' });

  useEffect(() => {
    let cancelled = false;

    fetchSystemDescription()
      .then((value) => {
        if (!cancelled) setState({ status: 'ready', value });
      })
      .catch((error: unknown) => {
        if (!cancelled) setState({ status: 'error', message: messageOf(error) });
      });

    return () => {
      cancelled = true;
    };
  }, []);

  return state;
}

/**
 * Follows the metric stream, keeping a bounded window of recent samples.
 *
 * @param windowSize How many samples to retain.
 * @returns Samples oldest first, and the most recent one for convenience.
 */
export function useMetrics(windowSize: number): {
  samples: MetricsSample[];
  latest: MetricsSample | null;
} {
  const [samples, setSamples] = useState<MetricsSample[]>([]);
  // Read inside the tick handler without making the subscription depend on it:
  // resubscribing on every window change would drop ticks during the swap.
  const limit = useRef(windowSize);

  useEffect(() => {
    limit.current = windowSize;
  }, [windowSize]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    fetchMetricsHistory(limit.current)
      .then((history) => {
        if (!cancelled) setSamples(history);
      })
      .catch(() => {
        // An empty chart is a better failure than a crashed page; the next tick
        // will start filling it regardless.
      });

    onMetricsTick((sample) => {
      setSamples((current) => [...current, sample].slice(-limit.current));
    })
      .then((stop) => {
        if (cancelled) stop();
        else unlisten = stop;
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // Trim immediately when the window shrinks, rather than waiting for ticks to
  // push the excess out.
  const trimmed = useMemo(
    () => (samples.length > windowSize ? samples.slice(-windowSize) : samples),
    [samples, windowSize]
  );

  return { samples: trimmed, latest: trimmed.at(-1) ?? null };
}

/**
 * Follows the process stream, applying each tick's diff to what is held.
 *
 * @returns The current tree and how many processes it holds.
 */
export function useProcesses(): { tree: ProcessTree; loaded: boolean } {
  const [records, setRecords] = useState<ReadonlyMap<string, ProcessRecord> | null>(null);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    fetchProcessList()
      .then((list) => {
        if (cancelled) return;
        setRecords(new Map(list.map((record) => [keyOf(record), record])));
      })
      .catch(() => {});

    onProcessesTick((diff) => {
      setRecords((current) => (current === null ? current : applyDiff(current, diff)));
    })
      .then((stop) => {
        if (cancelled) stop();
        else unlisten = stop;
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const tree = useMemo(() => buildTree(records === null ? [] : [...records.values()]), [records]);

  return { tree, loaded: records !== null };
}

/**
 * Loads the GPU list, retrying once the probe reports it has finished.
 *
 * @returns `null` while probing, then the devices found — possibly none.
 */
export function useGpuDevices(): GpuDevice[] | null {
  const [devices, setDevices] = useState<GpuDevice[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    const load = (): void => {
      fetchGpuDevices()
        .then((found) => {
          if (!cancelled && found !== null) setDevices(found);
        })
        .catch(() => {});
    };

    load();
    onGpusReady(load)
      .then((stop) => {
        if (cancelled) stop();
        else unlisten = stop;
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  return devices;
}

/**
 * Whether the window has been hidden to the tray at least once this session.
 *
 * Deliberately in-memory only: it seeds from nothing and flips true only on
 * the runtime event, never from storage. A launch that follows an earlier
 * hide must start `false` again — that hide happened to a window this
 * process never had, so there is nothing this session to explain.
 *
 * @returns Whether a hide-to-tray has happened since this page loaded.
 */
export function useTrayHidden(): boolean {
  const [hidden, setHidden] = useState(false);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    onTrayHidden(() => {
      setHidden(true);
    })
      .then((stop) => {
        if (cancelled) stop();
        else unlisten = stop;
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  return hidden;
}
