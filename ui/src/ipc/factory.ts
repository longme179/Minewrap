/**
 * IPC adapter selection: Tauri in the desktop webview, mock in a plain
 * browser (Vite dev / screenshots). The production adapter is in
 * `src/ipc/tauri.ts`; the mock is in `src/ipc/mock.ts`.
 */

import type { IpcAdapter } from "./mod.js";
import { MockIpcAdapter } from "./mock.js";
import { TauriIpcAdapter } from "./tauri.js";

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function createAdapter(): IpcAdapter {
  return isTauri() ? new TauriIpcAdapter() : new MockIpcAdapter();
}
