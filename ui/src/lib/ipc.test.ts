import { beforeEach, describe, expect, it, vi } from 'vitest';
import { COMMANDS, fetchAppInfo } from './ipc';

const invoke = vi.hoisted(() => vi.fn());

vi.mock('@tauri-apps/api/core', () => ({ invoke }));

describe('ipc', () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it('uses the snake_case command name the Rust handler registers', async () => {
    invoke.mockResolvedValue({});

    await fetchAppInfo();

    expect(invoke).toHaveBeenCalledWith('app_info');
    expect(COMMANDS.appInfo).toBe('app_info');
  });

  it('propagates backend errors to the caller', async () => {
    invoke.mockRejectedValue(new Error('command not allowed'));

    await expect(fetchAppInfo()).rejects.toThrow('command not allowed');
  });
});
