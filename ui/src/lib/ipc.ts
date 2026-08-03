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
import type { ChatComplete } from '../bindings/ChatComplete';
import type { ChatFailure } from '../bindings/ChatFailure';
import type { ChatToken } from '../bindings/ChatToken';
import type { CloseBehaviour } from '../bindings/CloseBehaviour';
import type { Conversation } from '../bindings/Conversation';
import type { CriticalProcess } from '../bindings/CriticalProcess';
import type { GpuDevice } from '../bindings/GpuDevice';
import type { LlmAdvice } from '../bindings/LlmAdvice';
import type { LogLevel } from '../bindings/LogLevel';
import type { UiEventKind } from '../bindings/UiEventKind';
import type { MetricsSample } from '../bindings/MetricsSample';
import type { InstalledRuntimeInfo } from '../bindings/InstalledRuntimeInfo';
import type { LibraryMovePlan } from '../bindings/LibraryMovePlan';
import type { ModelCatalogueEntry } from '../bindings/ModelCatalogueEntry';
import type { ModelDone } from '../bindings/ModelDone';
import type { ModelFailure } from '../bindings/ModelFailure';
import type { ModelProgress } from '../bindings/ModelProgress';
import type { ModelRegistry } from '../bindings/ModelRegistry';
import type { ModelSession } from '../bindings/ModelSession';
import type { SearchedFit } from '../bindings/SearchedFit';
import type { SearchResult } from '../bindings/SearchResult';
import type { PortRecord } from '../bindings/PortRecord';
import type { ProcessDiff } from '../bindings/ProcessDiff';
import type { ProcessKey } from '../bindings/ProcessKey';
import type { ProcessRecord } from '../bindings/ProcessRecord';
import type { Termination } from '../bindings/Termination';
import type { TerminationMode } from '../bindings/TerminationMode';
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
  terminateProcess: 'terminate_process',
  criticalProcesses: 'critical_processes',
  chatOpenModel: 'chat_open_model',
  chatStatus: 'chat_status',
  chatSend: 'chat_send',
  chatStop: 'chat_stop',
  chatClose: 'chat_close',
  chatList: 'chat_list',
  chatLoad: 'chat_load',
  chatDelete: 'chat_delete',
  modelsCatalogue: 'models_catalogue',
  modelsDownload: 'models_download',
  modelsSearch: 'models_search',
  modelsPriceSearched: 'models_price_searched',
  modelsDownloadSearched: 'models_download_searched',
  modelsPause: 'models_pause',
  modelsCancel: 'models_cancel',
  modelsDelete: 'models_delete',
  modelsFolder: 'models_folder',
  modelsSetFolder: 'models_set_folder',
  modelsPlanMove: 'models_plan_move',
  modelsMove: 'models_move',
  logSetLevel: 'log_set_level',
  logSave: 'log_save',
  logDirectory: 'log_directory',
  uiLog: 'ui_log',
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
  chatToken: 'chat:token',
  chatComplete: 'chat:complete',
  chatFailed: 'chat:failed',
  modelProgress: 'model:progress',
  modelDone: 'model:done',
  modelFailed: 'model:failed',
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
 * Opens a model file and starts a server for it.
 *
 * The result describes the model and what the launch settled on — never the
 * port, the base URL or the session key. Those stay in Rust, which is the whole
 * of ADR-012's argument: a compromised webview has no address to reach and no
 * key to reach it with.
 *
 * @param path An absolute path to a GGUF file.
 */
export function chatOpenModel(path: string): Promise<ModelSession> {
  return invoke<ModelSession>(COMMANDS.chatOpenModel, { path });
}

/**
 * The session Rust has open right now, or `null` when it has none.
 *
 * Rust owns the session, not the page: it is opened by the Run control on the
 * LLM tab before the chat page exists, and it survives that page being
 * unmounted and mounted again. So the page asks rather than assumes — an
 * assumption is how a model bar ends up drawn for a server that has stopped.
 */
export function chatStatus(): Promise<ModelSession | null> {
  return invoke<ModelSession | null>(COMMANDS.chatStatus);
}

/**
 * Sends one message and starts a reply.
 *
 * Resolves as soon as the work is spawned, not when the reply finishes: a long
 * answer takes tens of seconds and would otherwise hold the command channel the
 * rest of the app shares. The reply arrives on {@link onChatToken},
 * {@link onChatComplete} and {@link onChatFailed}.
 *
 * @param conversationId Which conversation the message belongs to.
 * @param text What the user typed.
 */
export async function chatSend(conversationId: string, text: string): Promise<void> {
  await invoke(COMMANDS.chatSend, { conversationId, text });
}

/** Stops the reply currently streaming. The partial text is kept. */
export async function chatStop(): Promise<void> {
  await invoke(COMMANDS.chatStop);
}

/** Closes the open model and ends its server. Closing nothing succeeds. */
export async function chatClose(): Promise<void> {
  await invoke(COMMANDS.chatClose);
}

/** Every stored conversation, oldest first. */
export function chatList(): Promise<Conversation[]> {
  return invoke<Conversation[]>(COMMANDS.chatList);
}

/**
 * Reads one stored conversation.
 *
 * @param id The conversation's identifier.
 */
export function chatLoad(id: string): Promise<Conversation> {
  return invoke<Conversation>(COMMANDS.chatLoad, { id });
}

/**
 * Deletes one stored conversation, removing its file.
 *
 * @param id The conversation's identifier.
 */
export async function chatDelete(id: string): Promise<void> {
  await invoke(COMMANDS.chatDelete, { id });
}

/** Subscribes to each piece of text the model produces. */
export function onChatToken(handler: (token: ChatToken) => void): Promise<UnlistenFn> {
  return listen<ChatToken>(EVENTS.chatToken, (event) => {
    handler(event.payload);
  });
}

/** Subscribes to the end of a reply, however it ended. */
export function onChatComplete(handler: (complete: ChatComplete) => void): Promise<UnlistenFn> {
  return listen<ChatComplete>(EVENTS.chatComplete, (event) => {
    handler(event.payload);
  });
}

/**
 * Subscribes to a reply that could not be finished.
 *
 * The payload carries the server's own words where it left any — an
 * out-of-memory message is written to stderr and nowhere else.
 */
export function onChatFailed(handler: (failure: ChatFailure) => void): Promise<UnlistenFn> {
  return listen<ChatFailure>(EVENTS.chatFailed, (event) => {
    handler(event.payload);
  });
}

/**
 * Every fit-matrix cell that has a pinned file, a downloaded one, or both.
 *
 * A cell missing from this list is one nobody has pinned. That absence is what
 * lets the advisor say "not pinned" rather than offer a control that would
 * fail, so the caller must not fill a gap with a default entry.
 */
export function fetchModelCatalogue(): Promise<ModelCatalogueEntry[]> {
  return invoke<ModelCatalogueEntry[]>(COMMANDS.modelsCatalogue);
}

/**
 * Starts downloading the file pinned for one cell.
 *
 * Resolves as soon as the work is spawned, not when the file lands — a model is
 * gigabytes. Progress and the outcome arrive on {@link onModelProgress},
 * {@link onModelDone} and {@link onModelFailed}.
 *
 * Rejects before any request is made if the cell has no pinned file, another
 * download is already running, or there is not enough room.
 *
 * @param modelId The model's registry id, e.g. `qwen2.5-7b`.
 * @param quantId The quantization id, e.g. `Q4_K_M`.
 */
export async function downloadModel(modelId: string, quantId: string): Promise<void> {
  await invoke(COMMANDS.modelsDownload, { modelId, quantId });
}

/**
 * Searches Hugging Face for downloadable GGUF files.
 *
 * The webview makes no request of its own here — it hands over a term and gets
 * back results. Every byte still moves in Rust, which is what keeps the CSP an
 * unweakened control (SECURITY.md threat 3).
 *
 * Results are a **weaker verification tier** than the pinned seven: the hash
 * they carry comes from the same origin as the file, so checking it detects a
 * corrupted transfer and not a replaced upload. Anything rendering these must
 * say so — see {@link downloadSearchedModel}.
 *
 * Rejects if the term is empty or Hugging Face could not be reached. A response
 * that will not parse resolves to an empty list rather than rejecting.
 *
 * @param query What to search for.
 */
export function searchModels(query: string): Promise<SearchResult[]> {
  return invoke<SearchResult[]>(COMMANDS.modelsSearch, { query });
}

/**
 * Prices one searched result, having read its header off the front of the file.
 *
 * A GGUF header sits at the start of the file, so Rust fetches it with a `Range`
 * request and hands it to the same launch arithmetic a downloaded model gets.
 * **The verdict is therefore the real one, not an estimate** — there is one
 * calculator in the codebase and this uses it.
 *
 * One network request against a multi-gigabyte file, so call it **on demand**
 * for the result somebody asked about, never in a loop over a page of them.
 *
 * Rejects if the GPU probe has not finished, the file could not be reached, or
 * its header did not arrive within the ceiling Rust reads to — which is what a
 * server that ignores `Range` produces. A rejection means "unpriced": show the
 * size alone and say why, rather than guessing a verdict from it.
 *
 * @param result A result {@link searchModels} returned, unaltered. Rust checks
 *   it again rather than trusting it, so an edited one is refused.
 */
export function priceSearchedModel(result: SearchResult): Promise<SearchedFit> {
  return invoke<SearchedFit>(COMMANDS.modelsPriceSearched, { result });
}

/**
 * Starts downloading a searched model.
 *
 * The same transfer as {@link downloadModel} in every respect that matters —
 * pause, resume, the bounded backoff, the free-space check, and the same
 * refusal on a hash that does not match. What differs is what gets recorded:
 * the model is marked `searched`, and the model list says so.
 *
 * Resolves as soon as the work is spawned. Progress and the outcome arrive on
 * {@link onModelProgress}, {@link onModelDone} and {@link onModelFailed}.
 *
 * @param result A result {@link searchModels} returned, unaltered. Rust checks
 *   it again rather than trusting it, so an edited one is refused.
 */
export async function downloadSearchedModel(result: SearchResult): Promise<void> {
  await invoke(COMMANDS.modelsDownloadSearched, { result });
}

/**
 * Pauses the download currently running, if there is one.
 *
 * The partial file is **kept**, so asking for the same file again resumes from
 * where it stopped rather than from zero. That is the only thing separating
 * this from {@link cancelModelDownload}.
 */
export async function pauseModelDownload(): Promise<void> {
  await invoke(COMMANDS.modelsPause);
}

/**
 * Cancels the download currently running, if there is one.
 *
 * The partial file is **deleted**, so asking for the same file again starts
 * from zero. That is the only thing separating this from
 * {@link pauseModelDownload} — deliberately, because abandoning a download and
 * silently leaving several gigabytes behind is the opposite of what a
 * disk-cleaning utility is for.
 */
export async function cancelModelDownload(): Promise<void> {
  await invoke(COMMANDS.modelsCancel);
}

/**
 * Deletes a downloaded model: the file, and the record naming it.
 *
 * @param modelId The model's registry id.
 * @param quantId The quantization id.
 */
export async function deleteModel(modelId: string, quantId: string): Promise<void> {
  await invoke(COMMANDS.modelsDelete, { modelId, quantId });
}

/** Where downloaded models are kept. Asking creates nothing. */
export function fetchModelFolder(): Promise<string> {
  return invoke<string>(COMMANDS.modelsFolder);
}

/**
 * Chooses where downloaded models are kept from now on.
 *
 * Changes nothing already on disk — moving what is there is
 * {@link moveModelLibrary}, a separate action because it can take an hour.
 *
 * @param path An absolute path to a folder.
 */
export async function setModelFolder(path: string): Promise<void> {
  await invoke(COMMANDS.modelsSetFolder, { path });
}

/**
 * States what moving the library to `path` would cost, without moving it.
 *
 * Settings asks this before it asks the user: "move 3 files, 14.2 GB" is a
 * question somebody can answer, and "change the model folder?" is not.
 *
 * @param path An absolute path to the folder the library would move to.
 */
export function planModelMove(path: string): Promise<LibraryMovePlan> {
  return invoke<LibraryMovePlan>(COMMANDS.modelsPlanMove, { path });
}

/**
 * Moves every downloaded model into `path`, and points new downloads there.
 *
 * Resolves as soon as the work is spawned, for the same reason
 * {@link downloadModel} does. Progress and the outcome arrive on the same three
 * subscriptions, with a `null` key because a move is not about one cell.
 *
 * @param path An absolute path to the folder to move into.
 */
export async function moveModelLibrary(path: string): Promise<void> {
  await invoke(COMMANDS.modelsMove, { path });
}

/** Subscribes to download and move progress. */
export function onModelProgress(handler: (progress: ModelProgress) => void): Promise<UnlistenFn> {
  return listen<ModelProgress>(EVENTS.modelProgress, (event) => {
    handler(event.payload);
  });
}

/** Subscribes to the one-shot signal that a download or a move finished. */
export function onModelDone(handler: (done: ModelDone) => void): Promise<UnlistenFn> {
  return listen<ModelDone>(EVENTS.modelDone, (event) => {
    handler(event.payload);
  });
}

/**
 * Subscribes to a download or move that could not be finished.
 *
 * The payload distinguishes four things a single message could not: a checksum
 * mismatch (`verificationFailure`, never retried), a transport failure that
 * survived the backoff (`retryable`), and the user stopping it — `stopped`,
 * which is not a failure at all, and which says whether the partial file was
 * kept (`'pause'`) or deleted (`'cancel'`).
 */
export function onModelFailed(handler: (failure: ModelFailure) => void): Promise<UnlistenFn> {
  return listen<ModelFailure>(EVENTS.modelFailed, (event) => {
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
 * Sets how much detail the log carries, from now on.
 *
 * Takes effect immediately and without a restart. Re-applied at startup the
 * same way {@link setCloseBehaviour} is, and for the same reason: the value
 * lives in the front-end's preferences and Rust holds only what it was last
 * told.
 *
 * @param level How much detail to capture.
 */
export async function setLogLevel(level: LogLevel): Promise<void> {
  await invoke(COMMANDS.logSetLevel, { level });
}

/** Where the log files are kept. Deleting that folder is safe at any time. */
export function fetchLogDirectory(): Promise<string> {
  return invoke<string>(COMMANDS.logDirectory);
}

/**
 * Copies every log file into `path`.
 *
 * @param path An absolute path to the folder to copy them into.
 * @returns How many files were copied.
 */
export function saveLogs(path: string): Promise<number> {
  return invoke<number>(COMMANDS.logSave, { path });
}

/**
 * Reports something the front end did into the one log Rust writes.
 *
 * A command rather than a file the webview keeps itself, so a session reads as
 * a single ordered story instead of two files somebody has to interleave by
 * timestamp. The parameter is one of a fixed set of kinds and never a string:
 * a free-text parameter would be somewhere a page title or a message could be
 * put, and the log carries no user data at any level.
 *
 * @param kind What happened.
 */
export async function logUiEvent(kind: UiEventKind): Promise<void> {
  await invoke(COMMANDS.uiLog, { kind });
}

/**
 * Ends a process the current user owns.
 *
 * Takes the whole {@link ProcessKey}, not a PID. Several seconds pass between
 * the graceful and forceful steps and a PID freed in that window can be
 * reused, so the start time is what stops the second call landing on a
 * process nobody selected.
 *
 * @param key Which process, identified by PID *and* start time.
 * @param mode Whether to ask it to exit or end it outright.
 */
export function terminateProcess(key: ProcessKey, mode: TerminationMode): Promise<Termination> {
  return invoke<Termination>(COMMANDS.terminateProcess, { key, mode });
}

/** Returns the processes on this platform that need a second confirmation. */
export function fetchCriticalProcesses(): Promise<CriticalProcess[]> {
  return invoke<CriticalProcess[]>(COMMANDS.criticalProcesses);
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
