/**
 * Console ring buffer store (Svelte 5). The frontend owns the 1000-line
 * ring; Rust keeps only a small buffer for Discord/backfill. Appends
 * never re-render the whole list: subscribers get incremental lines.
 */

import { writable } from "svelte/store";

import type { IpcAdapter, LogBatch, LogLine } from "../ipc/mod.js";

export const DEFAULT_RING = 1000;

export interface ConsoleState {
  lines: LogLine[];
  firstSeq: number;
  lastSeq: number;
  /** True when auto-scroll is stuck to the bottom. */
  stick: boolean;
  /** Total lines ever seen (for "jump to latest" counts). */
  total: number;
}

const initial: ConsoleState = {
  lines: [],
  firstSeq: 0,
  lastSeq: 0,
  stick: true,
  total: 0,
};

export function createConsoleStore(ringSize: number = DEFAULT_RING) {
  const { subscribe, update, set } = writable<ConsoleState>(initial);
  let unsubscribe: (() => void) | null = null;

  const api = {
    subscribe,
    set,

    /** Subscribe to log batches from a backend adapter. */
    attach(adapter: IpcAdapter): void {
      api.detach();
      unsubscribe = adapter.onLogsBatch((batch: LogBatch) => {
        api.append(batch.lines);
      });
    },

    detach(): void {
      unsubscribe?.();
      unsubscribe = null;
    },

    append(lines: LogLine[]): void {
      if (lines.length === 0) return;
      update((s) => {
        const next = s.lines.concat(lines);
        const trimmed = next.length > ringSize ? next.slice(next.length - ringSize) : next;
        return {
          lines: trimmed,
          firstSeq: trimmed[0]?.seq ?? 0,
          lastSeq: trimmed[trimmed.length - 1]?.seq ?? s.lastSeq,
          stick: s.stick,
          total: s.total + lines.length,
        };
      });
    },

    /** Dedupes against the current ring by seq (v1 ConsoleModel parity). */
    appendBatch(lines: LogLine[]): void {
      const filtered = lines.filter((l) => l.seq > 0);
      if (filtered.length === 0) return;
      update((s) => {
        const seen = new Set(s.lines.map((l) => l.seq));
        const fresh = filtered.filter((l) => !seen.has(l.seq));
        if (fresh.length === 0) return s;
        const next = s.lines.concat(fresh);
        const trimmed = next.length > ringSize ? next.slice(next.length - ringSize) : next;
        return {
          lines: trimmed,
          firstSeq: trimmed[0]?.seq ?? 0,
          lastSeq: trimmed[trimmed.length - 1]?.seq ?? s.lastSeq,
          stick: s.stick,
          total: s.total + fresh.length,
        };
      });
    },

    setStick(stick: boolean): void {
      update((s) => ({ ...s, stick }));
    },

    clear(): void {
      set(initial);
    },
  };

  return api;
}

/** Parse a Minecraft log level from a line (`[Server thread/INFO]:`). */
export function levelOf(line: string): "trace" | "debug" | "info" | "warn" | "error" | "fatal" | "unknown" {
  const m = line.match(/\/(TRACE|DEBUG|INFO|WARN|ERROR|FATAL)\]/);
  if (!m) return "unknown";
  switch (m[1]) {
    case "TRACE":
      return "trace";
    case "DEBUG":
      return "debug";
    case "WARN":
      return "warn";
    case "ERROR":
      return "error";
    case "FATAL":
      return "fatal";
    default:
      return "info";
  }
}

/** Highlight helper: returns a CSS class per level using design tokens. */
export function levelClass(level: ReturnType<typeof levelOf>): string {
  switch (level) {
    case "trace":
      return "level-trace";
    case "debug":
      return "level-debug";
    case "warn":
      return "level-warn";
    case "error":
      return "level-error";
    case "fatal":
      return "level-fatal";
    default:
      return "level-info";
  }
}
