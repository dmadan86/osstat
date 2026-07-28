import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from './App';
import type { AppInfo } from './bindings/AppInfo';

const invoke = vi.hoisted(() => vi.fn());

vi.mock('@tauri-apps/api/core', () => ({ invoke }));

const SAMPLE: AppInfo = {
  build: { name: 'osstat', version: '0.1.0', profile: 'debug' },
  platform: 'linux',
  platformName: 'Linux',
};

describe('App', () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it('shows a loading state before the backend answers', () => {
    invoke.mockReturnValue(new Promise(() => {}));

    render(<App />);

    expect(screen.getByRole('status')).toBeInTheDocument();
  });

  it('renders the version, platform and build profile once loaded', async () => {
    invoke.mockResolvedValue(SAMPLE);

    render(<App />);

    expect(await screen.findByText('0.1.0')).toBeInTheDocument();
    expect(screen.getByText('Linux')).toBeInTheDocument();
    expect(screen.getByText('debug')).toBeInTheDocument();
  });

  it('calls the app_info command exactly once', async () => {
    invoke.mockResolvedValue(SAMPLE);

    render(<App />);
    await screen.findByText('0.1.0');

    expect(invoke).toHaveBeenCalledWith('app_info');
  });

  it('surfaces a backend failure instead of hanging on the loading state', async () => {
    invoke.mockRejectedValue(new Error('ipc unavailable'));

    render(<App />);

    expect(await screen.findByRole('alert')).toHaveTextContent('ipc unavailable');
  });

  it('lists the capabilities that are not implemented yet', () => {
    invoke.mockReturnValue(new Promise(() => {}));

    render(<App />);

    expect(screen.getByRole('heading', { name: /not built yet/i })).toBeInTheDocument();
    expect(screen.getByText('Cleaner')).toBeInTheDocument();
  });
});
