/**
 * Mock IPC adapter for browser (Vite) runs — fake console data so the
 * console component can be exercised at 900×600 / 1280 / 1920 without
 * Tauri. The production adapter is in `src/ipc/tauri.ts`.
 */

import type {
  Action,
  DiscordTestResponse,
  InstanceSummary,
  IpcAdapter,
  LogBatch,
  LogLine,
  ModDto,
  NodeInput,
  NodeSummary,
  NodeId,
  PropertiesDoc,
  SnapshotDto,
  StateChanged,
} from "./mod.js";

const MOCK_LINES: string[] = [
  "[01:48:51] [Server thread/INFO]: Starting minecraft server version 1.21.4",
  "[01:48:51] [Server thread/INFO]: Loading properties",
  "[01:48:52] [Server thread/INFO]: Default game type: SURVIVAL",
  "[01:48:52] [Server thread/WARN]: **** SERVER IS RUNNING IN OFFLINE/INSECURE MODE!",
  "[01:48:52] [Server thread/WARN]: The server will make no attempt to obtain a new identity from MinecraftDirect. All players will be assumed to be connecting clients regardless of their username.",
  "[01:48:53] [Server thread/INFO]: Preparing level \"world\"",
  "[01:48:54] [Server thread/INFO]: Preparing spawn area: 100%",
  "[01:48:55] [Server thread/INFO]: Time elapsed: 842 ms",
  "[01:48:55] [Server thread/INFO]: Done (2.341s)! For help, type \"help\"",
  "[01:49:01] [Server thread/INFO]: Alice[/127.0.0.1:56789] logged in with entity id 1 at (100.5, 64.0, 200.5)",
  "[01:49:03] [Server thread/INFO]: Alice joined the game",
  "[01:49:10] [Server thread/WARN]: Can't keep up! Is the server overloaded? Running 2047ms or 40 ticks behind",
];

let seq = 0;
let timer: ReturnType<typeof setInterval> | null = null;
const batchListeners = new Set<(b: LogBatch) => void>();
const stateListeners = new Set<(s: StateChanged) => void>();

function fakeLine(i: number): LogLine {
  const raw = MOCK_LINES[i % MOCK_LINES.length] ?? `line ${i}`;
  return { seq: ++seq, ts: new Date().toISOString(), line: raw };
}

export class MockIpcAdapter implements IpcAdapter {
  private running = false;

  async listNodes(): Promise<NodeSummary[]> {
    return [
      {
        id: "local",
        name: "This machine",
        url: "local",
        is_local: true,
        state: "connected",
      },
    ];
  }

  async addNode(input: NodeInput): Promise<NodeSummary> {
    return {
      id: input.url.replace(/^https?:\/\//, ""),
      name: input.name,
      url: input.url,
      is_local: false,
      state: "connected",
    };
  }

  async removeNode(_nodeId: NodeId): Promise<Action> {
    return { ok: true };
  }

  async listInstances(_nodeId: NodeId): Promise<InstanceSummary[]> {
    return [
      {
        id: "survival",
        name: "Survival",
        combined_state: "online",
        players_online: 1,
        players_max: 20,
        version: "1.21.4",
        latency_ms: 3.2,
      },
    ];
  }

  async getSnapshot(_nodeId: NodeId, instanceId: string): Promise<SnapshotDto> {
    return {
      id: instanceId,
      name: "Survival",
      combined_state: "online",
      process_state: "online",
      probe_ok: true,
      players_online: 1,
      players_max: 20,
      version: "1.21.4",
    };
  }

  async instanceStart(_n: NodeId, _i: string): Promise<Action> {
    return { ok: true, detail: "start requested" };
  }
  async instanceStop(_n: NodeId, _i: string): Promise<Action> {
    return { ok: true, detail: "stop requested" };
  }
  async instanceRestart(_n: NodeId, _i: string): Promise<Action> {
    return { ok: true, detail: "restart requested" };
  }
  async sendCommand(_n: NodeId, _i: string, _c: string): Promise<Action> {
    return { ok: true, detail: "command sent" };
  }

  async getLogs(_n: NodeId, _i: string, limit = 200): Promise<LogLine[]> {
    const out: LogLine[] = [];
    for (let k = 0; k < Math.min(limit, MOCK_LINES.length); k++) out.push(fakeLine(k));
    return out;
  }

  async readProperties(): Promise<PropertiesDoc> {
    return {
      text: "#Minecraft server properties\nserver-port=25565\nmotd=A Minecraft Server\nmax-players=20\n",
      errors: [],
    };
  }

  async writeProperties(): Promise<Action> {
    return { ok: true, detail: "properties written" };
  }

  async listMods(): Promise<ModDto[]> {
    return [
      { name: "sodium-0.6.13.jar", size: 912_000, enabled: true },
      { name: "lithium-0.14.1.jar.disabled", size: 640_000, enabled: false },
    ];
  }

  async toggleMod(): Promise<Action> {
    return { ok: true, detail: "toggled" };
  }

  async discordTest(): Promise<DiscordTestResponse> {
    return { ok: true, error: null };
  }

  async health(): Promise<Action> {
    return { ok: true, detail: "1.0" };
  }

  onLogsBatch(cb: (b: LogBatch) => void): () => void {
    batchListeners.add(cb);
    if (!this.running) {
      this.running = true;
      let n = 0;
      timer = setInterval(() => {
        const lines: LogLine[] = Array.from({ length: 3 }, () => fakeLine(n++));
        const batch: LogBatch = {
          node_id: "local",
          instance_id: "survival",
          lines,
          first_seq: lines[0]?.seq ?? 0,
          last_seq: lines[lines.length - 1]?.seq ?? 0,
        };
        for (const l of batchListeners) l(batch);
      }, 800);
    }
    return () => {
      batchListeners.delete(cb);
    };
  }

  onStateChanged(cb: (s: StateChanged) => void): () => void {
    stateListeners.add(cb);
    return () => {
      stateListeners.delete(cb);
    };
  }
}

export function __clearMockTimer(): void {
  if (timer) {
    clearInterval(timer);
    timer = null;
  }
  batchListeners.clear();
  stateListeners.clear();
}
