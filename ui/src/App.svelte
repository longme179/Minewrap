<!-- Minewrap v2 shell: 900x600 minimum, dark-only, token-driven.
  Wired to the IPC adapter (Tauri in the webview, mock in a plain
  browser) — see `src/main.ts`. -->
<script lang="ts">
  import Dashboard from "./components/Dashboard.svelte";
  import AddInstance from "./components/AddInstance.svelte";
  import { createAdapter } from "./ipc/factory.js";
  import { createConsoleStore } from "./stores/console.js";

  const ipc = createAdapter();
  const store = createConsoleStore();

  let nodes = $state<{ id: string; name: string; state: string }[]>([]);
  let instances = $state<{ id: string; name: string; combined_state: string }[]>([]);
  let snapshot = $state<null | {
    id: string; name: string; combined_state: string;
    players_online?: number | null; players_max?: number | null;
    version?: string | null; probe_ok: boolean;
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

  async function start() { await ipc.instanceStart(selectedNode, selectedInstance); await loadInstances(); }
  async function stop() { await ipc.instanceStop(selectedNode, selectedInstance); await loadInstances(); }
  async function restart() { await ipc.instanceRestart(selectedNode, selectedInstance); await loadInstances(); }

  async function remove() {
    await ipc.removeInstance(selectedNode, selectedInstance);
    selectedInstance = "";
    await loadInstances();
  }

  function selectInstance(id: string) {
    selectedInstance = id;
    view = "tree";
    void loadInstances();
  }

  $effect(() => {
    store.attach(ipc);
    void load();
    return () => store.detach();
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
      {#if selectedInstance}
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
      {:else if view === "add"}
        <AddInstance onChanged={() => void loadInstances()} />
      {:else}
        <p class="empty">Select an instance, or add one.</p>
      {/if}
      {#if view === "add"}
        <AddInstance onChanged={() => { view = "tree"; void loadInstances(); }} />
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
