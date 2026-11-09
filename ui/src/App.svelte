<!-- Minewrap v2 shell: 900x600 minimum, dark-only, token-driven. -->
<script lang="ts">
  import Console from "./components/Console.svelte";
  import { createConsoleStore } from "./stores/console.js";

  const store = createConsoleStore();
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
        <li>
          <span class="dot dot-online" aria-hidden="true"></span>
          This machine — online
        </li>
      </ul>
      <p class="tree-title">Instances</p>
      <ul>
        <li><strong>Survival</strong> <span class="badge">online</span></li>
      </ul>
    </nav>
    <section class="page" aria-label="Instance view">
      <div class="status">
        <span class="dot dot-online" aria-hidden="true"></span>
        <span>Online · 1/20 players · 1.21.4 · 3 ms</span>
      </div>
      <Console {store} />
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
  }
  .tree-title {
    color: var(--color-muted-foreground);
    text-transform: uppercase;
    font-size: 11px;
    margin: var(--space-lg) 0 var(--space-sm);
  }
  .tree ul { list-style: none; padding: 0; margin: 0; }
  .tree li { padding: var(--space-sm) 0; }
  .page { flex: 1; display: flex; flex-direction: column; min-width: 0; }
  .status {
    padding: var(--space-md) var(--space-lg);
    display: flex;
    gap: var(--space-md);
    align-items: center;
    border-bottom: 1px solid var(--color-border);
  }
  .dot { width: 8px; height: 8px; border-radius: 50%; display: inline-block; }
  .dot-online { background: var(--color-state-online); }
  .badge {
    background: var(--color-muted);
    border-radius: var(--radius-sm);
    padding: 0 var(--space-sm);
    font-size: 11px;
  }
</style>
