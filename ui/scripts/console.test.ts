/// <reference types="vitest" />
import { describe, it, expect } from "vitest";
import { createConsoleStore, levelOf, DEFAULT_RING } from "../src/stores/console.js";
import { MockIpcAdapter } from "../src/ipc/mock.js";

describe("levelOf", () => {
  it("maps minecraft levels", () => {
    expect(levelOf("[01:48:51] [Server thread/INFO]: hi")).toBe("info");
    expect(levelOf("[01:48:51] [Server thread/WARN]: hi")).toBe("warn");
    expect(levelOf("[01:48:51] [main/ERROR]: hi")).toBe("error");
    expect(levelOf("plain line")).toBe("unknown");
  });
});

describe("console store ring", () => {
  it("caps at the ring size and preserves monotonic seq", async () => {
    const store = createConsoleStore(5);
    const adapter = new MockIpcAdapter();
    const lines = await adapter.getLogs("local", "survival", 12);
    store.append(lines);
    let snap: { lines: { seq: number }[] } | null = null;
    const un = store.subscribe((v) => (snap = v));
    un();
    expect(snap!.lines.length).toBeLessThanOrEqual(5);
    expect(snap!.lines[0]?.seq).toBeGreaterThan(0);
  });

  it("dedupes backfilled lines by seq", () => {
    const store = createConsoleStore(DEFAULT_RING);
    store.appendBatch([
      { seq: 1, ts: new Date().toISOString(), line: "a" },
      { seq: 2, ts: new Date().toISOString(), line: "b" },
    ]);
    store.appendBatch([
      { seq: 2, ts: new Date().toISOString(), line: "b" },
      { seq: 3, ts: new Date().toISOString(), line: "c" },
    ]);
    let snap: { lines: { seq: number }[]; total: number } | null = null;
    const un = store.subscribe((v) => (snap = v));
    un();
    expect(snap!.lines.map((l) => l.seq)).toEqual([1, 2, 3]);
    expect(snap!.total).toBe(3);
  });
});
