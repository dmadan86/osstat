/**
 * The Overview page: what this machine is, and what it is doing right now.
 *
 * Each section is an independent component, which is what lets the page layout
 * preference be a container decision rather than a rewrite.
 */

import type { GpuDevice } from '../bindings/GpuDevice';
import type { MetricsSample } from '../bindings/MetricsSample';
import type { SystemDescription } from '../bindings/SystemDescription';
import { Chart } from '../components/Chart';
import { Meter } from '../components/Meter';
import { chartHeightFor } from '../components/PanelGrid';
import { SectionContainer, type SectionSpec } from '../components/Section';
import {
  memoryAreaOption,
  percentAreaOption,
  perCoreHeatmapOption,
  throughputOption,
  type TimePoint,
} from '../charts/options';
import {
  formatBytes,
  formatDuration,
  formatFrequency,
  formatPercent,
  formatRate,
} from '../lib/format';
import { reconcileLayout, type PanelLayout } from '../lib/panelLayout';
import type { PageLayout } from '../lib/preferences';
import { OVERVIEW_PANEL_IDS } from './overviewPanels';

/** What the Overview page needs. */
export interface OverviewProps {
  /** The machine's identity. */
  system: SystemDescription;
  /** Recent samples, oldest first. */
  samples: MetricsSample[];
  /** The most recent sample, if any. */
  latest: MetricsSample | null;
  /** GPUs found, or `null` while the probe is still running. */
  gpus: GpuDevice[] | null;
  /** How to present the sections. */
  layout: PageLayout;
  /** The stored panel arrangement. */
  panels: PanelLayout[];
  /** Applies an arrangement change. */
  onPanelsChange: (next: PanelLayout[]) => void;
  /** Whether to explain that closing the window did not quit. */
  showTrayNotice: boolean;
  /** Records that the explanation has been read. */
  onTrayNoticeSeen: () => void;
}

/** A labelled figure. */
function Fact({ label, value }: { label: string; value: string }): React.JSX.Element {
  return (
    <div className="flex justify-between gap-4 py-0.5 text-sm">
      <span className="text-neutral-500">{label}</span>
      <span data-selectable className="font-mono text-neutral-200">
        {value}
      </span>
    </div>
  );
}

/** Says a section has nothing to show yet, without pretending it is empty. */
function Waiting({ children }: { children: string }): React.JSX.Element {
  return (
    <p role="status" className="py-6 text-center text-sm text-neutral-500">
      {children}
    </p>
  );
}

/** Builds the Overview sections. */
function sectionsFor(props: OverviewProps): SectionSpec[] {
  const { system, samples, latest, gpus, panels } = props;

  const reconciled = reconcileLayout(panels, OVERVIEW_PANEL_IDS);
  const heightOf = (id: string): number => chartHeightFor(reconciled, id);

  const cpuPoints: TimePoint[] = samples.map((sample) => ({
    at: sample.atMs,
    value: sample.cpuTotal,
  }));

  const memoryPoints: TimePoint[] = samples.map((sample) => ({
    at: sample.atMs,
    value: sample.memoryUsed,
  }));

  // Network is charted for the busiest interface rather than summed: adding a
  // loopback and a Wi-Fi adapter together produces a number that describes
  // nothing real.
  const busiest = latest?.interfaces.reduce<{ index: number; total: number } | null>(
    (best, entry) => {
      const total = entry.rxPerSecond + entry.txPerSecond;
      return best === null || total > best.total ? { index: entry.index, total } : best;
    },
    null
  );
  const interfaceIndex = busiest?.index ?? latest?.interfaces[0]?.index ?? 0;
  const rxPoints: TimePoint[] = samples.map((sample) => ({
    at: sample.atMs,
    value: sample.interfaces.find((entry) => entry.index === interfaceIndex)?.rxPerSecond ?? 0,
  }));
  const txPoints: TimePoint[] = samples.map((sample) => ({
    at: sample.atMs,
    value: sample.interfaces.find((entry) => entry.index === interfaceIndex)?.txPerSecond ?? 0,
  }));
  const interfaceName =
    system.interfaces.find((entry) => entry.index === interfaceIndex)?.name ?? 'Network';

  const memoryTotal = system.totalMemory;
  const memoryUsed = latest?.memoryUsed ?? 0;

  return [
    {
      id: 'cpu',
      title: 'CPU',
      summary: latest === null ? undefined : formatPercent(latest.cpuTotal),
      content: (
        <div className="grid gap-4 lg:grid-cols-[1fr_200px]">
          <div className="min-w-0">
            {cpuPoints.length === 0 ? (
              <Waiting>Waiting for the first sample…</Waiting>
            ) : (
              <Chart
                option={percentAreaOption(cpuPoints, 'CPU')}
                height={heightOf('cpu')}
                label="Total CPU utilisation over time"
              />
            )}

            {latest !== null && latest.cpuPerCore.length > 0 && (
              <div className="mt-3">
                <p className="mb-1.5 text-[10px] uppercase tracking-wider text-neutral-600">
                  Per core — {latest.cpuPerCore.length} logical
                </p>
                <Chart
                  option={perCoreHeatmapOption(latest.cpuPerCore)}
                  height={Math.ceil(latest.cpuPerCore.length / 16) * 30 + 8}
                  label="Utilisation of each logical core"
                />
              </div>
            )}
          </div>

          <div>
            <Fact label="Model" value={system.cpu.brand || '—'} />
            <Fact
              label="Cores"
              value={
                system.cpu.physicalCores === null
                  ? String(system.cpu.logicalCores)
                  : `${system.cpu.physicalCores} / ${system.cpu.logicalCores}`
              }
            />
            <Fact label="Frequency" value={formatFrequency(system.cpu.frequencyMhz)} />
            <Fact label="Vendor" value={system.cpu.vendor || '—'} />
          </div>
        </div>
      ),
    },
    {
      id: 'memory',
      title: 'Memory',
      summary:
        latest === null ? undefined : `${formatBytes(memoryUsed)} / ${formatBytes(memoryTotal)}`,
      content: (
        <div className="grid gap-4 lg:grid-cols-[1fr_200px]">
          <div className="min-w-0">
            {memoryPoints.length === 0 ? (
              <Waiting>Waiting for the first sample…</Waiting>
            ) : (
              <Chart
                option={memoryAreaOption(memoryPoints, memoryTotal)}
                height={heightOf('memory')}
                label="Memory in use over time"
              />
            )}
          </div>
          <div>
            <Fact label="Total" value={formatBytes(memoryTotal)} />
            <Fact label="Available" value={formatBytes(latest?.memoryAvailable ?? 0)} />
            <Fact
              label="Swap"
              value={`${formatBytes(latest?.swapUsed ?? 0)} / ${formatBytes(system.totalSwap)}`}
            />
          </div>
        </div>
      ),
    },
    {
      id: 'disks',
      title: 'Disks',
      summary: `${system.disks.length} mounted`,
      content:
        system.disks.length === 0 ? (
          <Waiting>No mounted volumes were reported.</Waiting>
        ) : (
          <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
            {system.disks.map((disk) => {
              const usage = latest?.disks.find((entry) => entry.index === disk.index);
              const total = usage?.total ?? 0;
              const used = usage?.used ?? 0;
              return (
                <Meter
                  key={disk.index}
                  fraction={total > 0 ? used / total : 0}
                  label={`${disk.name}${disk.kind === 'unknown' ? '' : ` · ${disk.kind.toUpperCase()}`}`}
                  detail={`${formatBytes(used)} used of ${formatBytes(total)}`}
                  warnWhenFull
                />
              );
            })}
          </div>
        ),
    },
    {
      id: 'network',
      title: 'Network',
      summary:
        latest === null
          ? undefined
          : `↓ ${formatRate(rxPoints.at(-1)?.value ?? 0)}  ↑ ${formatRate(txPoints.at(-1)?.value ?? 0)}`,
      content: (
        <div className="grid gap-4 lg:grid-cols-[1fr_200px]">
          <div className="min-w-0">
            {rxPoints.length === 0 ? (
              <Waiting>Waiting for the first sample…</Waiting>
            ) : (
              <>
                <Chart
                  option={throughputOption(rxPoints, txPoints)}
                  height={heightOf('network')}
                  label={`Throughput on ${interfaceName}`}
                />
                <div className="mt-1.5 flex gap-4 text-[11px] text-neutral-500">
                  <span className="flex items-center gap-1.5">
                    <span className="h-2 w-2 rounded-sm bg-[#3987e5]" aria-hidden="true" />
                    Down
                  </span>
                  <span className="flex items-center gap-1.5">
                    <span className="h-2 w-2 rounded-sm bg-[#d95926]" aria-hidden="true" />
                    Up
                  </span>
                </div>
              </>
            )}
          </div>
          <div>
            <Fact label="Interface" value={interfaceName} />
            <Fact
              label="Received"
              value={formatBytes(
                latest?.interfaces.find((entry) => entry.index === interfaceIndex)?.rxTotal ?? 0
              )}
            />
            <Fact
              label="Sent"
              value={formatBytes(
                latest?.interfaces.find((entry) => entry.index === interfaceIndex)?.txTotal ?? 0
              )}
            />
          </div>
        </div>
      ),
    },
    {
      id: 'gpu',
      title: 'GPU',
      summary: gpus === null ? 'detecting…' : gpus.length === 0 ? 'none found' : undefined,
      content: <GpuContent gpus={gpus} latest={latest} />,
    },
  ];
}

/** The GPU section body. */
function GpuContent({
  gpus,
  latest,
}: {
  gpus: GpuDevice[] | null;
  latest: MetricsSample | null;
}): React.JSX.Element {
  if (gpus === null) return <Waiting>Looking for graphics adapters…</Waiting>;
  if (gpus.length === 0) {
    return <Waiting>No GPU was found. That is normal on a headless or virtual machine.</Waiting>;
  }

  return (
    <div className="flex flex-col gap-4">
      {gpus.map((gpu) => {
        const sample = latest?.gpus.find((entry) => entry.index === gpu.index);
        const measured = gpu.source === 'nvml';

        return (
          // Constrained: a fact row stretched across a wide window puts its
          // label and its value at opposite ends of the screen, which is
          // unreadable and, past a certain width, pushes the value out of view.
          <div key={gpu.index} className="flex max-w-xl flex-col gap-2">
            <div className="flex items-baseline justify-between gap-3">
              <span data-selectable className="text-sm">
                {gpu.name}
              </span>
              <span className="font-mono text-xs text-neutral-500">
                {gpu.kind}
                {gpu.backend === null ? '' : ` · ${gpu.backend}`}
              </span>
            </div>

            {gpu.vramTotal === null ? (
              // wgpu has no API that reports video memory. Saying so is better
              // than a number invented from a device limit -- ADR-008 names
              // presenting a heuristic as a measurement the worst thing this
              // feature could do.
              <p className="text-xs text-neutral-500">
                Video memory is not reported by this source
                {gpu.backend === null ? '' : ` (${gpu.backend})`}.
              </p>
            ) : (
              <Meter
                fraction={
                  sample?.vramUsed != null && gpu.vramTotal > 0
                    ? sample.vramUsed / gpu.vramTotal
                    : 0
                }
                label={
                  gpu.source === 'unifiedMemory'
                    ? 'Unified memory (shared with the system)'
                    : 'Video memory'
                }
                detail={
                  sample?.vramUsed == null
                    ? `${formatBytes(gpu.vramTotal)} total${measured ? '' : ' — estimated'}`
                    : `${formatBytes(sample.vramUsed)} of ${formatBytes(gpu.vramTotal)}${
                        measured ? '' : ' — estimated'
                      }`
                }
              />
            )}

            <div className="grid grid-cols-2 gap-x-6">
              <Fact
                label="Utilisation"
                value={
                  sample?.utilisation == null ? 'not measurable' : formatPercent(sample.utilisation)
                }
              />
              <Fact
                label="Temperature"
                value={
                  sample?.temperatureC == null
                    ? 'not measurable'
                    : `${Math.round(sample.temperatureC)} °C`
                }
              />
            </div>
          </div>
        );
      })}
    </div>
  );
}

/**
 * Renders the Overview page.
 *
 * @param props The system description, samples, GPUs and layout preference.
 */
export function Overview(props: OverviewProps): React.JSX.Element {
  const { system } = props;

  return (
    <div className="flex flex-col gap-4">
      {props.showTrayNotice && (
        <div
          role="status"
          aria-label="osstat keeps running in the notification area"
          className="flex items-start gap-3 rounded-lg border border-edge bg-surface-raised px-4 py-2.5 text-xs text-neutral-300"
        >
          <span className="flex-1">
            osstat kept running in the notification area when you closed the window. You can change
            that in Settings.
          </span>
          <button
            type="button"
            aria-label="Dismiss"
            onClick={props.onTrayNoticeSeen}
            className="text-neutral-500 hover:text-neutral-200"
          >
            ✕
          </button>
        </div>
      )}

      <header className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <h2 data-selectable className="text-lg font-semibold">
          {system.hostName ?? 'This machine'}
        </h2>
        <p data-selectable className="text-xs text-neutral-500">
          {[system.osName, system.osVersion, system.kernelVersion].filter(Boolean).join(' · ')}
          {' · up '}
          {formatDuration(system.uptimeSeconds)}
        </p>
      </header>

      <SectionContainer
        sections={sectionsFor(props)}
        layout={props.layout}
        panels={reconcileLayout(props.panels, OVERVIEW_PANEL_IDS)}
        onPanelsChange={props.onPanelsChange}
      />
    </div>
  );
}
