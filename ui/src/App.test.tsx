import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { App } from './App';
import type { GpuDevice } from './bindings/GpuDevice';
import type { MetricsSample } from './bindings/MetricsSample';
import type { ProcessRecord } from './bindings/ProcessRecord';
import type { SystemDescription } from './bindings/SystemDescription';

const invoke = vi.hoisted(() => vi.fn());
const listen = vi.hoisted(() => vi.fn());

vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen }));

// ECharts draws to a canvas, which jsdom does not implement. The option
// builders are pure functions tested directly in charts/options.test.ts, so
// nothing is lost by standing the renderer down here.
vi.mock('./components/Chart', () => ({
  Chart: ({ label }: { label: string }) => <div role="img" aria-label={label} />,
}));

const SYSTEM: SystemDescription = {
  hostName: 'TESTBOX',
  osName: 'Test OS',
  osVersion: '1.0',
  kernelVersion: '9.9',
  uptimeSeconds: 2 * 86_400 + 4 * 3_600,
  cpu: {
    brand: 'Test CPU X1',
    vendor: 'TestVendor',
    physicalCores: 8,
    logicalCores: 16,
    frequencyMhz: 4500,
  },
  totalMemory: 34_359_738_368,
  totalSwap: 8_589_934_592,
  disks: [
    { index: 0, name: 'C:', mountPoint: 'C:\\', fileSystem: 'NTFS', kind: 'ssd', removable: false },
  ],
  interfaces: [{ index: 0, name: 'Wi-Fi', mac: null, addresses: ['192.168.1.24'] }],
};

const SAMPLE: MetricsSample = {
  atMs: 1_700_000_000_000,
  cpuTotal: 23.4,
  cpuPerCore: Array.from({ length: 16 }, () => 20),
  memoryUsed: 12_000_000_000,
  memoryAvailable: 22_000_000_000,
  swapUsed: 0,
  disks: [{ index: 0, used: 600_000_000_000, total: 1_000_000_000_000 }],
  interfaces: [{ index: 0, rxPerSecond: 1200, txPerSecond: 340, rxTotal: 5000, txTotal: 900 }],
  gpus: [],
};

const PROCESSES: ProcessRecord[] = [
  {
    pid: 1,
    parentPid: null,
    name: 'chrome',
    exe: null,
    user: 'tester',
    cpu: 12,
    memory: 3_000_000_000,
    diskReadRate: 0,
    diskWriteRate: 0,
    startedAt: 1000,
    status: 'running',
  },
  {
    pid: 2,
    parentPid: 1,
    name: 'renderer',
    exe: null,
    user: 'tester',
    cpu: 5,
    memory: 700_000_000,
    diskReadRate: 0,
    diskWriteRate: 0,
    startedAt: 1000,
    status: 'running',
  },
];

const GPU: GpuDevice = {
  index: 0,
  name: 'Test GPU 5000',
  vendor: 'NVIDIA',
  backend: 'NVML',
  kind: 'discrete',
  vramTotal: 8_589_934_592,
  source: 'nvml',
};

/** Routes each command to its canned answer. */
function respond(overrides: Record<string, unknown> = {}): void {
  invoke.mockImplementation((command: string) => {
    const answers: Record<string, unknown> = {
      system_description: SYSTEM,
      metrics_history: [SAMPLE],
      process_list: PROCESSES,
      gpu_devices: [GPU],
      set_sample_interval: undefined,
      set_sampling_paused: undefined,
      ...overrides,
    };
    return Promise.resolve(answers[command]);
  });
}

describe('App', () => {
  beforeEach(() => {
    invoke.mockReset();
    listen.mockReset();
    listen.mockResolvedValue(() => {});
    window.localStorage.clear();
    respond();
  });

  it('shows a loading state before the backend answers', () => {
    invoke.mockReturnValue(new Promise(() => {}));

    render(<App />);

    expect(screen.getByRole('status')).toBeInTheDocument();
  });

  it('surfaces a backend failure instead of hanging on the loading state', async () => {
    invoke.mockRejectedValue(new Error('ipc unavailable'));

    render(<App />);

    expect(await screen.findByRole('alert')).toHaveTextContent('ipc unavailable');
  });

  it('shows the machine it is running on', async () => {
    render(<App />);

    expect(await screen.findByText('TESTBOX')).toBeInTheDocument();
    expect(screen.getByText(/Test OS/)).toBeInTheDocument();
    expect(screen.getByText(/2 d 4 h/)).toBeInTheDocument();
  });

  it('shows real CPU details from the description', async () => {
    render(<App />);

    expect(await screen.findByText('Test CPU X1')).toBeInTheDocument();
    expect(screen.getByText('8 / 16')).toBeInTheDocument();
    expect(screen.getByText('4.50 GHz')).toBeInTheDocument();
  });

  it('subscribes to the live streams rather than polling', async () => {
    render(<App />);
    await screen.findByText('TESTBOX');

    const events = listen.mock.calls.map((call) => call[0] as string);
    expect(events).toContain('metrics:tick');
    expect(events).toContain('processes:tick');
  });

  it('navigates to the process tree and rolls a subtree up onto its collapsed parent', async () => {
    render(<App />);
    await screen.findByText('TESTBOX');

    await userEvent.click(screen.getByTitle('Processes'));

    expect(await screen.findByText('chrome')).toBeInTheDocument();
    // Collapsed, chrome accounts for both processes rather than only its own 12%.
    expect(screen.getByText('17.0%')).toBeInTheDocument();
  });

  it('expands a process to reveal its children', async () => {
    render(<App />);
    await screen.findByText('TESTBOX');
    await userEvent.click(screen.getByTitle('Processes'));
    await screen.findByText('chrome');

    expect(screen.queryByText('renderer')).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: /Expand chrome/ }));

    expect(await screen.findByText('renderer')).toBeInTheDocument();
  });

  it('filters the tree and reveals matches inside collapsed parents', async () => {
    render(<App />);
    await screen.findByText('TESTBOX');
    await userEvent.click(screen.getByTitle('Processes'));

    await userEvent.type(await screen.findByLabelText('Filter processes'), 'renderer');

    expect(await screen.findByText('renderer')).toBeInTheDocument();
  });

  it('shows what an unbuilt page will do rather than an empty screen', async () => {
    render(<App />);
    await screen.findByText('TESTBOX');

    await userEvent.click(screen.getByTitle(/Cleaner/));

    expect(await screen.findByText('System cleaner')).toBeInTheDocument();
    expect(screen.getByText(/previewed|preview/i)).toBeInTheDocument();
    // "M3" appears on the nav badge as well as the placeholder.
    expect(screen.getAllByText('M3').length).toBeGreaterThan(0);
  });

  it('switches navigation style from settings and keeps every page reachable', async () => {
    render(<App />);
    await screen.findByText('TESTBOX');

    await userEvent.click(screen.getByTitle('Settings'));
    await userEvent.click(await screen.findByRole('radio', { name: 'Top tabs' }));

    await userEvent.click(screen.getByTitle('Processes'));
    expect(await screen.findByLabelText('Filter processes')).toBeInTheDocument();
  });

  it('persists the chosen layout', async () => {
    render(<App />);
    await screen.findByText('TESTBOX');

    await userEvent.click(screen.getByTitle('Settings'));
    await userEvent.click(await screen.findByRole('radio', { name: 'Icon rail' }));

    await waitFor(() => {
      expect(window.localStorage.getItem('osstat.preferences.v1')).toContain('rail');
    });
  });

  it('tells the backend when the refresh interval changes', async () => {
    render(<App />);
    await screen.findByText('TESTBOX');

    await userEvent.click(screen.getByTitle('Settings'));
    await userEvent.click(await screen.findByRole('radio', { name: '1 second' }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('set_sample_interval', { millis: 1000 });
    });
  });

  it('pauses sampling rather than asking for a zero interval', async () => {
    render(<App />);
    await screen.findByText('TESTBOX');

    await userEvent.click(screen.getByTitle('Settings'));
    await userEvent.click(await screen.findByRole('radio', { name: 'Paused' }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('set_sampling_paused', { paused: true });
    });
  });

  it('presents sections as sub-tabs when that layout is chosen', async () => {
    render(<App />);
    await screen.findByText('TESTBOX');

    await userEvent.click(screen.getByTitle('Settings'));
    await userEvent.click(await screen.findByRole('radio', { name: 'Sub-tabs' }));
    await userEvent.click(screen.getByTitle('Overview'));

    expect(await screen.findByRole('tab', { name: 'CPU' })).toBeInTheDocument();
  });

  it('does not claim a VRAM figure that wgpu cannot supply', async () => {
    respond({ gpu_devices: [{ ...GPU, source: 'wgpu', vramTotal: null, backend: 'Vulkan' }] });
    render(<App />);
    await screen.findByText('TESTBOX');

    expect(await screen.findByText(/not reported by this source/)).toBeInTheDocument();
  });

  it('distinguishes still-probing from no GPU present', async () => {
    respond({ gpu_devices: null });
    render(<App />);
    await screen.findByText('TESTBOX');

    expect(await screen.findByText(/Looking for graphics adapters/)).toBeInTheDocument();
  });

  it('says so plainly when the machine has no GPU', async () => {
    respond({ gpu_devices: [] });
    render(<App />);
    await screen.findByText('TESTBOX');

    expect(await screen.findByText(/No GPU was found/)).toBeInTheDocument();
  });

  it('explains the tray only after a hide actually happens, not on a fresh launch', async () => {
    render(<App />);
    await screen.findByText('TESTBOX');

    expect(screen.queryByRole('status', { name: /notification area/i })).not.toBeInTheDocument();

    const [, onHidden] =
      listen.mock.calls.find((call) => call[0] === 'tray:hidden') ??
      ([undefined, undefined] as const);
    expect(onHidden).toBeDefined();
    await act(async () => {
      onHidden?.({});
    });

    expect(await screen.findByRole('status', { name: /notification area/i })).toBeInTheDocument();
  });

  it('offers to end a process, and ends nothing without a confirmation', async () => {
    // Replaces "offers no way to kill a process in this phase". The absence was
    // pinned deliberately, so what replaces it is pinned deliberately too.
    render(<App />);
    await screen.findByText('TESTBOX');
    await userEvent.click(screen.getByTitle('Processes'));
    await screen.findByText('chrome');

    const [endButton] = screen.getAllByRole('button', { name: /end .*chrome/i });
    expect(endButton).toBeDefined();
    await userEvent.click(endButton as HTMLElement);

    expect(await screen.findByRole('dialog', { name: /end chrome/i })).toBeInTheDocument();
    expect(invoke).not.toHaveBeenCalledWith('terminate_process', expect.anything());
  });
});
