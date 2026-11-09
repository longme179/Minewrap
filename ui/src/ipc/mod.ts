/**
 * IPC adapter contract — mirrors the Tauri command/event table in
 * ARCHITECTURE.md. The frontend talks only to this interface; there is
 * no local/remote branch in UI code and tokens never enter the webview.
 */

export type NodeId = string;

export interface NodeSummary {
  id: NodeId;
  name: string;
  url: string;
  is_local: boolean;
  state: "connecting" | "connected" | "offline" | "unauthorised" | "incompatible";
}

export interface InstanceSummary {
  id: string;
  name: string;
  combined_state: string;
  players_online?: number | null;
  players_max?: number | null;
  version?: string | null;
  latency_ms?: number | null;
}

export interface SnapshotDto {
  id: string;
  name: string;
  combined_state: string;
  process_state: string;
  probe_ok: boolean;
  players_online?: number | null;
  players_max?: number | null;
  version?: string | null;
}

export interface LogLine {
  seq: number;
  ts: string; // ISO 8601 (chrono ts_milliseconds)
  line: string;
}

export interface Action {
  ok: boolean;
  detail?: string | null;
}

export interface DiscordTestResponse {
  ok: boolean;
  error?: string | null;
}

export interface PropertiesDoc {
  text: string;
  errors: string[];
}

export interface ModDto {
  name: string;
  size: number;
  enabled: boolean;
}

export interface MinewrapErrorShape {
  code: string;
  message: string;
}

export class MinewrapError extends Error {
  code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

export interface LogBatch {
  node_id: NodeId;
  instance_id: string;
  lines: LogLine[];
  first_seq: number;
  last_seq: number;
}

export interface StateChanged {
  node_id: NodeId;
  instance_id: string;
  old_state: string;
  new_state: string;
  reason: string;
}

export interface NodeInput {
  name: string;
  url: string;
  token: string;
}

export interface IpcAdapter {
  listNodes(): Promise<NodeSummary[]>;
  addNode(input: NodeInput): Promise<NodeSummary>;
  removeNode(nodeId: NodeId): Promise<Action>;
  updateNode?(nodeId: NodeId, input: NodeInput): Promise<NodeSummary>;
  listInstances(nodeId: NodeId): Promise<InstanceSummary[]>;
  getSnapshot(nodeId: NodeId, instanceId: string): Promise<SnapshotDto>;
  instanceStart(nodeId: NodeId, instanceId: string): Promise<Action>;
  instanceStop(nodeId: NodeId, instanceId: string): Promise<Action>;
  instanceRestart(nodeId: NodeId, instanceId: string): Promise<Action>;
  sendCommand(nodeId: NodeId, instanceId: string, command: string): Promise<Action>;
  getLogs(nodeId: NodeId, instanceId: string, limit?: number, sinceSeq?: number): Promise<LogLine[]>;
  readProperties(nodeId: NodeId, instanceId: string): Promise<PropertiesDoc>;
  writeProperties(nodeId: NodeId, instanceId: string, doc: PropertiesDoc): Promise<Action>;
  listMods(nodeId: NodeId, instanceId: string): Promise<ModDto[]>;
  toggleMod(nodeId: NodeId, instanceId: string, name: string): Promise<Action>;
  discordTest(nodeId: NodeId, instanceId: string): Promise<DiscordTestResponse>;
  health(nodeId: NodeId): Promise<Action>;

  /** Subscribe to batched log events. Returns an unsubscribe function. */
  onLogsBatch(cb: (batch: LogBatch) => void): () => void;
  /** Subscribe to state changes. Returns an unsubscribe function. */
  onStateChanged(cb: (change: StateChanged) => void): () => void;
}
