/**
 * Tauri IPC adapter: invokes the commands registered in `src-tauri` and
 * listens to the event stream. This is the production adapter; the
 * browser mock lives in `src/ipc/mock.ts`.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type {
  Action,
  DiscordTestResponse,
  InstanceSummary,
  IpcAdapter,
  LogBatch,
  LogLine,
  ModDto,
  NodeId,
  NodeInput,
  NodeSummary,
  PropertiesDoc,
  SnapshotDto,
  StateChanged,
} from "./mod.js";

export class TauriIpcAdapter implements IpcAdapter {
  async listNodes(): Promise<NodeSummary[]> {
    return invoke<NodeSummary[]>("list_nodes");
  }

  async addNode(input: NodeInput): Promise<NodeSummary> {
    return invoke<NodeSummary>("add_node", { input });
  }

  async removeNode(nodeId: NodeId): Promise<Action> {
    return invoke<Action>("remove_node", { nodeId });
  }

  async listInstances(nodeId: NodeId): Promise<InstanceSummary[]> {
    return invoke<InstanceSummary[]>("list_instances", { nodeId });
  }

  async getSnapshot(nodeId: NodeId, instanceId: string): Promise<SnapshotDto> {
    return invoke<SnapshotDto>("get_snapshot", { nodeId, instanceId });
  }

  async instanceStart(nodeId: NodeId, instanceId: string): Promise<Action> {
    return invoke<Action>("instance_start", { nodeId, instanceId });
  }

  async instanceStop(nodeId: NodeId, instanceId: string): Promise<Action> {
    return invoke<Action>("instance_stop", { nodeId, instanceId });
  }

  async instanceRestart(nodeId: NodeId, instanceId: string): Promise<Action> {
    return invoke<Action>("instance_restart", { nodeId, instanceId });
  }

  async sendCommand(nodeId: NodeId, instanceId: string, command: string): Promise<Action> {
    return invoke<Action>("send_command", { nodeId, instanceId, command });
  }

  async getLogs(nodeId: NodeId, instanceId: string, limit = 200, sinceSeq = 0): Promise<LogLine[]> {
    return invoke<LogLine[]>("get_logs", { nodeId, instanceId, limit, sinceSeq });
  }

  async readProperties(nodeId: NodeId, instanceId: string): Promise<PropertiesDoc> {
    return invoke<PropertiesDoc>("read_properties", { nodeId, instanceId });
  }

  async writeProperties(nodeId: NodeId, instanceId: string, doc: PropertiesDoc): Promise<Action> {
    return invoke<Action>("write_properties", { nodeId, instanceId, doc });
  }

  async listMods(nodeId: NodeId, instanceId: string): Promise<ModDto[]> {
    return invoke<ModDto[]>("list_mods", { nodeId, instanceId });
  }

  async toggleMod(nodeId: NodeId, instanceId: string, name: string): Promise<Action> {
    return invoke<Action>("toggle_mod", { nodeId, instanceId, name });
  }

  async discordTest(nodeId: NodeId, instanceId: string): Promise<DiscordTestResponse> {
    return invoke<DiscordTestResponse>("discord_test", { nodeId, instanceId });
  }

  async health(nodeId: NodeId): Promise<Action> {
    return invoke<Action>("health", { nodeId });
  }

  onLogsBatch(cb: (b: LogBatch) => void): () => void {
    let off: (() => void) | null = null;
    void listen<LogBatch>("logs:batch", (e) => cb(e.payload)).then((f) => {
      off = f;
    });
    return () => off?.();
  }

  onStateChanged(cb: (s: StateChanged) => void): () => void {
    let off: (() => void) | null = null;
    void listen<StateChanged>("state:changed", (e) => cb(e.payload)).then((f) => {
      off = f;
    });
    return () => off?.();
  }
}
