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
import { disable, enable, isEnabled } from '@tauri-apps/plugin-autostart';

import type { AppInfo } from '../bindings/AppInfo';
import type { CloseBehaviour } from '../bindings/CloseBehaviour';
import type { GpuDevice } from '../bindings/GpuDevice';
import type { LlmAdvice } from '../bindings/LlmAdvice';
import type { MetricsSample } from '../bindings/MetricsSample';
import type { InstalledRuntimeInfo } from '../bindings/InstalledRuntimeInfo';
import type { ModelRegistry } from '../bindings/ModelRegistry';
import type { PortRecord } from '../bindings/PortRecord';
import type { ProcessDiff } from '../bindings/ProcessDiff';
import type { ProcessRecord } from '../bindings/ProcessRecord';
import type { RuntimeFailure } from '../bindings/RuntimeFailure';
import type { RuntimeProgress } from '../bindings/RuntimeProgress';
import type { RuntimeStatus } from '../bindings/RuntimeStatus';
import type { SystemDescription } from '../bindings/SystemDescription';

/** Names of every command the Rust side exposes. */
export const COMMANDS = {
  appInfo: 'app_info',
  systemDescription: 'system_description',
  metricsHistory: 'metrics_history',
  processList: 'process_list',
  portList: 'port_list',
  gpuDevices: 'gpu_devices',
  modelRegistry: 'model_registry',
  llmAdvice: 'llm_advice',
  runtimeStatus: 'runtime_status',
  acquireRuntime: 'acquire_runtime',
  deleteRuntime: 'delete_runtime',
  setSampleInterval: 'set_sample_interval',
  setSamplingPaused: 'set_sampling_paused',
  setCloseBehaviour: 'set_close_behaviour',
} as const;

/** Names of every event the Rust side emits. */
export const EVENTS = {
  metricsTick: 'metrics:tick',
  processesTick: 'processes:tick',
  gpusReady: 'gpus:ready',
  trayHidden: 'tray:hidden',
  runtimeProgress: 'runtime:progress',
  runtimeReady: 'runtime:ready',
  runtimeFailed: 'runtime:failed',
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
 * Returns every socket currently open, joined to the process that holds it.
 *
 * Read fresh on every call — unlike the process table this is not fed by the
 * sampler's tick, so each call is a live enumeration as of that moment.
 */
export function fetchPortList(): Promise<PortRecord[]> {
  return invoke<PortRecord[]>(COMMANDS.portList);
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
 * Returns the seed model registry: the quantization levels and the models.
 *
 * Static data embedded in the binary, so this is fetched once and held rather
 * than re-read whenever the context length changes.
 */
export function fetchModelRegistry(): Promise<ModelRegistry> {
  return invoke<ModelRegistry>(COMMANDS.modelRegistry);
}

/**
 * Weighs every model at every quantization against this machine.
 *
 * Returns `null` while the GPU probe is still running, the same convention
 * {@link fetchGpuDevices} uses: a matrix computed before the probe answers
 * would report "no GPU" on a machine that has one. The `gpus:ready` event is
 * the cue to ask again.
 *
 * @param contextLength Tokens of context to price the KV cache at. Clamped
 *   by the backend; the figure actually used comes back on the result.
 */
export function fetchLlmAdvice(contextLength: number): Promise<LlmAdvice | null> {
  return invoke<LlmAdvice | null>(COMMANDS.llmAdvice, { contextLength });
}

/**
 * Returns what inference runtimes are installed and what could be installed,
 * or `null` while the GPU probe is still running.
 *
 * `null` for the same reason {@link fetchLlmAdvice} uses it: recommending a CPU
 * build on a machine that has a GPU is a confident wrong answer. The
 * `gpus:ready` event is the cue to ask again.
 */
export function fetchRuntimeStatus(): Promise<RuntimeStatus | null> {
  return invoke<RuntimeStatus | null>(COMMANDS.runtimeStatus);
}

/**
 * Starts downloading and verifying a llama.cpp runtime.
 *
 * Resolves as soon as the work is spawned, not when it completes — a Windows
 * CUDA acquisition moves 642 MB. Progress and the outcome arrive on
 * {@link onRuntimeProgress}, {@link onRuntimeReady} and {@link onRuntimeFailed}.
 *
 * @param acceptCuda Whether the user accepted CUDA's much larger download.
 *   Defaults to the smaller Vulkan build, because a 642 MB download is a
 *   choice rather than a consequence of owning an NVIDIA card.
 */
export async function acquireRuntime(acceptCuda: boolean): Promise<void> {
  await invoke(COMMANDS.acquireRuntime, { acceptCuda });
}

/**
 * Deletes an installed runtime.
 *
 * @param tag The upstream release tag it came from.
 * @param artifactId Which build, as named in `runtimes.json`.
 */
export async function deleteRuntime(tag: string, artifactId: string): Promise<void> {
  await invoke(COMMANDS.deleteRuntime, { tag, artifactId });
}

/** Subscribes to download and verification progress. */
export function onRuntimeProgress(
  handler: (progress: RuntimeProgress) => void
): Promise<UnlistenFn> {
  return listen<RuntimeProgress>(EVENTS.runtimeProgress, (event) => {
    handler(event.payload);
  });
}

/** Subscribes to the one-shot signal that a runtime is installed and usable. */
export function onRuntimeReady(
  handler: (runtime: InstalledRuntimeInfo) => void
): Promise<UnlistenFn> {
  return listen<InstalledRuntimeInfo>(EVENTS.runtimeReady, (event) => {
    handler(event.payload);
  });
}

/**
 * Subscribes to acquisition failures.
 *
 * The payload carries `retryable` and `verificationFailure` rather than only a
 * message: a checksum mismatch must not be offered a retry, and must not read
 * like a bad day on the network.
 */
export function onRuntimeFailed(handler: (failure: RuntimeFailure) => void): Promise<UnlistenFn> {
  return listen<RuntimeFailure>(EVENTS.runtimeFailed, (event) => {
    handler(event.payload);
  });
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

/**
 * Tells the backend what closing the window should do.
 *
 * The decision has to be made synchronously inside Rust's close handler, so it
 * is pushed ahead of time rather than asked for at close time.
 *
 * @param behaviour Whether closing hides the window or exits.
 */
export async function setCloseBehaviour(behaviour: CloseBehaviour): Promise<void> {
  await invoke(COMMANDS.setCloseBehaviour, { behaviour });
}

/**
 * Whether the operating system is set to start osstat at sign-in.
 *
 * Behind this module for the same reason every command is: a plugin's API
 * crosses the same boundary an `invoke` does, and the webview's reach stays
 * auditable when it is all in one file.
 *
 * @returns What the OS currently has registered.
 */
export function isAutostartEnabled(): Promise<boolean> {
  return isEnabled();
}

/**
 * Registers or removes the sign-in entry.
 *
 * @param enabled Whether osstat should start at sign-in.
 */
export async function setAutostart(enabled: boolean): Promise<void> {
  await (enabled ? enable() : disable());
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

/**
 * Subscribes to the signal that the window was just hidden to the tray
 * rather than closed.
 *
 * This is the only way the front end learns a hide happened: the decision is
 * made inside Rust's close handler, synchronously, with nothing asked of the
 * webview beforehand.
 */
export function onTrayHidden(handler: () => void): Promise<UnlistenFn> {
  return listen(EVENTS.trayHidden, () => {
    handler();
  });
}
