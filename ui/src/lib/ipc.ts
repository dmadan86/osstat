/**
 * The typed edge of the IPC boundary.
 *
 * Every backend call goes through a named function here rather than a raw
 * `invoke` at the call site. Two reasons: the command-name strings stay in one
 * auditable place, and the return types come from `src/bindings/`, which is
 * generated from the Rust structs by `just bindings` (ADR-002). If a Rust type
 * changes without the frontend following, `npm run typecheck` fails.
 */
import { invoke } from '@tauri-apps/api/core';
import type { AppInfo } from '../bindings/AppInfo';

/** Names of every command the Rust side exposes. */
export const COMMANDS = {
  appInfo: 'app_info',
} as const;

/**
 * Returns the identity of the running application: name, version, build
 * profile and host platform.
 */
export function fetchAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>(COMMANDS.appInfo);
}
