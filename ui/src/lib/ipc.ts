/**
 * The typed edge of the IPC boundary.
 *
 * Every backend call goes through a named function here rather than a raw
 * `invoke` at the call site. Two reasons: the command-name strings stay in one
 * auditable place, and the return types come from `src/bindings/`, which is
 * generated from the Rust structs by `just bindings` (ADR-002). If a Rust type
 * changes without the frontend following, `npm run typecheck` fails.
 *
 * Event names live here for the same reason. A mistyped event name fails
 * silently — the listener simply never fires — so the strings belong beside the
 * commands, where a reader can check them against the Rust constants in
 * `src-tauri/src/sampler.rs` in one place.
 */
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

import type { AppInfo } from '../bindings/AppInfo';
import type { GpuDevice } from '../bindings/GpuDevice';
import type { MetricsSample } from '../bindings/MetricsSample';
import type { ProcessDiff } from '../bindings/ProcessDiff';
import type { ProcessRecord } from '../bindings/ProcessRecord';
import type { SystemDescription } from '../bindings/SystemDescription';

/** Names of every command the Rust side exposes. */
export const COMMANDS = {
  appInfo: 'app_info',
  systemDescription: 'system_description',
  metricsHistory: 'metrics_history',
  processList: 'process_list',
  gpuDevices: 'gpu_devices',
  setSampleInterval: 'set_sample_interval',
  setSamplingPaused: 'set_sampling_paused',
} as const;

/** Names of every event the Rust side emits. */
export const EVENTS = {
  metricsTick: 'metrics:tick',
  processesTick: 'processes:tick',
  gpusReady: 'gpus:ready',
} as const;

/**
 * Returns the identity of the running application: name, version, build
 * profile and host platform.
 */
export function fetchAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>(COMMANDS.appInfo);
}

/** Returns the machine's identity: OS, CPU, disks and interfaces. */
export function fetchSystemDescription(): Promise<SystemDescription> {
  return invoke<SystemDescription>(COMMANDS.systemDescription);
}

/**
 * Returns recent metric samples, oldest first.
 *
 * @param limit How many samples to return at most.
 */
export function fetchMetricsHistory(limit: number): Promise<MetricsSample[]> {
  return invoke<MetricsSample[]>(COMMANDS.metricsHistory, { limit });
}

/** Returns every process from the most recent tick, flat and unordered. */
export function fetchProcessList(): Promise<ProcessRecord[]> {
  return invoke<ProcessRecord[]>(COMMANDS.processList);
}

/**
 * Returns the GPUs found, or `null` while the probe is still running.
 *
 * `null` and `[]` mean different things: still looking, versus this machine has
 * no GPU at all. The UI says something different for each.
 */
export function fetchGpuDevices(): Promise<GpuDevice[] | null> {
  return invoke<GpuDevice[] | null>(COMMANDS.gpuDevices);
}

/**
 * Sets how often the backend samples.
 *
 * @param millis Milliseconds between ticks; `0` or less pauses sampling.
 */
export async function setSampleInterval(millis: number): Promise<void> {
  if (millis <= 0) {
    await invoke(COMMANDS.setSamplingPaused, { paused: true });
    return;
  }
  await invoke(COMMANDS.setSampleInterval, { millis });
}

/** Subscribes to per-tick metric samples. */
export function onMetricsTick(handler: (sample: MetricsSample) => void): Promise<UnlistenFn> {
  return listen<MetricsSample>(EVENTS.metricsTick, (event) => {
    handler(event.payload);
  });
}

/** Subscribes to per-tick process changes. */
export function onProcessesTick(handler: (diff: ProcessDiff) => void): Promise<UnlistenFn> {
  return listen<ProcessDiff>(EVENTS.processesTick, (event) => {
    handler(event.payload);
  });
}

/** Subscribes to the one-shot signal that GPU probing has finished. */
export function onGpusReady(handler: () => void): Promise<UnlistenFn> {
  return listen(EVENTS.gpusReady, () => {
    handler();
  });
}
