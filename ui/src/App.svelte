<!-- Minewrap v2 shell: 900x600 minimum, dark-only, token-driven.
  Wired to the IPC adapter (Tauri in the webview, mock in a plain
  browser) — see `src/main.ts`. -->
<script lang="ts">
  import Dashboard from "./components/Dashboard.svelte";
  import Wizard from "./components/Wizard.svelte";
  import { createAdapter } from "./ipc/factory.js";
  import { createConsoleStore } from "./stores/console.js";
  import { formatError } from "./lib/formatError.js";

  const ipc = createAdapter();
  const store = createConsoleStore();

  let nodes = $state<{ id: string; name: string; state: string }[]>([]);
  let instances = $state<{ id: string; name: string; combined_state: string }[]>([]);
  let snapshot = $state<null | {
    id: string; name: string; combined_state: string;
    players_online?: number | null; players_max?: number | null;
    version?: string | null; probe_ok: boolean; last_error?: string | null;
  }>(null);
  let nodeState = $state("connected");
  let selectedNode = $state("local");
  let selectedInstance = $state("survival");
  let tab = $state("console");
  let view = $state<"tree" | "add">("tree");

  async function load() {
    nodes = await ipc.listNodes();
    selectedNode = nodes[0]?.id ?? "local";
    nodeState = nodes[0]?.state ?? "offline";
    await loadInstances();
  }

  async function loadInstances() {
    instances = await ipc.listInstances(selectedNode);
    if (instances[0] && !instances.some((i) => i.id === selectedInstance)) {
      selectedInstance = instances[0].id;
    }
    snapshot = await ipc.getSnapshot(selectedNode, selectedInstance);
    await store.appendBatch(await ipc.getLogs(selectedNode, selectedInstance, 200));
  }

  let actionError = $state("");

  async function refreshSnapshot() {
    if (!selectedInstance) return;
    try {
      snapshot = await ipc.getSnapshot(selectedNode, selectedInstance);
      instances = instances.map((i) =>
        i.id === selectedInstance ? { ...i, combined_state: snapshot?.combined_state ?? i.combined_state } : i
      );
    } catch {
      // ignore transient refresh errors
    }
  }

  async function runAction(label: string, fn: () => Promise<unknown>) {
    actionError = "";
    try {
      await fn();
      await loadInstances();
      setTimeout(() => void refreshSnapshot(), 400);
      setTimeout(() => void refreshSnapshot(), 1500);
    } catch (e) {
      actionError = `${label} failed: ${formatError(e)}`;
    }
  }

  async function start() { await runAction("Start", () => ipc.instanceStart(selectedNode, selectedInstance)); }
  async function stop() { await runAction("Stop", () => ipc.instanceStop(selectedNode, selectedInstance)); }
  async function restart() { await runAction("Restart", () => ipc.instanceRestart(selectedNode, selectedInstance)); }

  async function remove() {
    actionError = "";
    try {
      await ipc.removeInstance(selectedNode, selectedInstance);
      selectedInstance = "";
      view = "tree";
      await loadInstances();
    } catch (e) {
      actionError = `Remove failed: ${formatError(e)}`;
    }
  }

  function selectInstance(id: string) {
    selectedInstance = id;
    view = "tree";
    void loadInstances();
  }

  $effect(() => {
    store.attach(ipc);
    void load();
    const off = ipc.onStateChanged((ev) => {
      if (ev.node_id === selectedNode && ev.instance_id === selectedInstance) {
        void refreshSnapshot();
      }
      instances = instances.map((i) =>
        i.id === ev.instance_id ? { ...i, combined_state: ev.new_state } : i
      );
    });
    const poll = setInterval(() => void refreshSnapshot(), 2000);
    return () => {
      store.detach();
      off();
      clearInterval(poll);
    };
  });
</script>

<main class="app">
  <header class="topbar">
    <h1>Minewrap</h1>
    <span class="subtitle">Desktop server manager</span>
  </header>
  <div class="layout">
    <nav aria-label="Instances" class="tree">
      <p class="tree-title">Nodes</p>
      <ul>
        {#each nodes as n (n.id)}
          <li>
            <button
              type="button"
              class="tree-item"
              class:tree-active={selectedNode === n.id}
              onclick={() => {
                selectedNode = n.id;
                void loadInstances();
              }}
            >
              <span class="dot dot-online" aria-hidden="true"></span>
              {n.name} — {n.state}
            </button>
          </li>
        {/each}
      </ul>
      <p class="tree-title">Instances</p>
      <ul>
        {#each instances as i (i.id)}
          <li>
            <button
              type="button"
              class="tree-item"
              class:tree-active={selectedInstance === i.id}
              onclick={() => selectInstance(i.id)}
            >
              <strong>{i.name}</strong>
              <span class="badge">{i.combined_state}</span>
            </button>
          </li>
        {/each}
      </ul>
      <div class="tree-actions">
        <button type="button" class="btn-secondary" onclick={() => (view = "add")}>+ Add instance</button>
        {#if selectedInstance}
          <button type="button" class="btn-secondary danger" onclick={remove}>Remove</button>
        {/if}
      </div>
    </nav>
    <section class="page" aria-label="Instance view">
      {#if actionError}
        <p role="alert" class="action-error">{actionError}</p>
      {/if}
      {#if view === "add"}
        <Wizard onChanged={() => { view = "tree"; actionError = ""; void loadInstances(); }} />
      {:else if selectedInstance}
        <Dashboard
          {snapshot}
          {nodeState}
          nodeId={selectedNode}
          instanceId={selectedInstance}
          bind:tab
          onStart={start}
          onStop={stop}
          onRestart={restart}
        />
      {:else}
        <p class="empty">Select an instance, or add one.</p>
      {/if}
    </section>
  </div>
</main>

<style>
  .app {
    min-width: 900px;
    min-height: 600px;
    height: 100vh;
    display: flex;
    flex-direction: column;
    background: var(--color-background);
    color: var(--color-foreground);
    font-family: var(--font-sans);
  }
  .topbar {
    display: flex;
    align-items: baseline;
    gap: var(--space-md);
    padding: var(--space-md) var(--space-lg);
    border-bottom: 1px solid var(--color-border);
  }
  h1 { font-family: var(--font-mono); font-size: 16px; margin: 0; }
  .subtitle { color: var(--color-muted-foreground); font-size: 12px; }
  .layout { flex: 1; display: flex; min-height: 0; }
  .tree {
    width: 220px;
    flex-shrink: 0;
    border-right: 1px solid var(--color-border);
    padding: var(--space-md);
    font-size: 13px;
    display: flex;
    flex-direction: column;
  }
  .tree-title {
    color: var(--color-muted-foreground);
    text-transform: uppercase;
    font-size: 11px;
    margin: var(--space-lg) 0 var(--space-sm);
  }
  .tree ul { list-style: none; padding: 0; margin: 0; }
  .tree li { padding: var(--space-sm) 0; }
  .tree-item {
    background: transparent;
    border: none;
    color: var(--color-foreground);
    font-size: 13px;
    cursor: pointer;
    text-align: left;
    width: 100%;
    padding: var(--space-sm) var(--space-md);
    border-radius: var(--radius-sm);
  }
  .tree-active { background: var(--color-muted); }
  .tree-actions { margin-top: auto; display: flex; flex-direction: column; gap: var(--space-sm); }
  .btn-secondary {
    background: transparent;
    color: var(--color-foreground);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: var(--space-sm) var(--space-md);
    font-weight: 600;
    cursor: pointer;
  }
  .btn-secondary.danger { color: var(--color-destructive); border-color: var(--color-destructive); }
  .page { flex: 1; display: flex; flex-direction: column; min-width: 0; overflow-y: auto; }
  .action-error {
    margin: var(--space-md) var(--space-lg) 0;
    padding: var(--space-sm) var(--space-md);
    border: 1px solid var(--color-destructive);
    border-radius: var(--radius-sm);
    color: var(--color-destructive);
  }
  .empty { color: var(--color-muted-foreground); padding: var(--space-lg); }
  .dot { width: 8px; height: 8px; border-radius: 50%; display: inline-block; }
  .dot-online { background: var(--color-state-online); }
  .badge {
    background: var(--color-muted);
    border-radius: var(--radius-sm);
    padding: 0 var(--space-sm);
    font-size: 11px;
    margin-left: var(--space-sm);
  }
</style>
