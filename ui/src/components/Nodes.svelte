<!-- Nodes page: node list with a local auto node + remotes,
  connection state, reconnect/backoff, unauthorised/incompatible banners.
  The local node is created automatically; remotes are added via URL +
  token (token never enters the webview over IPC — it is typed once and
  sent to Rust; Rust stores it in the keyring/0600 store). -->
<script lang="ts">
  import { createAdapter } from "../ipc/factory.js";
  import { formatError } from "../lib/formatError.js";
  import type { NodeSummary } from "../ipc/mod.js";

  const ipc = createAdapter();

  let nodes = $state<NodeSummary[]>([]);
  let newName = $state("");
  let newUrl = $state("");
  let newToken = $state("");
  let error = $state("");

  async function load() {
    nodes = await ipc.listNodes();
  }

  async function add() {
    error = "";
    if (!newUrl) {
      error = "URL is required.";
      return;
    }
    try {
      await ipc.addNode({ name: newName || newUrl, url: newUrl, token: newToken });
      newName = "";
      newUrl = "";
      newToken = "";
      await load();
    } catch (e) {
      error = formatError(e);
    }
  }

  async function retry(id: string) {
    error = "";
    try {
      await ipc.retryNode?.(id);
      await load();
    } catch (e) {
      error = formatError(e);
    }
  }

  async function remove(id: string) {
    try {
      await ipc.removeNode(id);
      await load();
    } catch (e) {
      error = formatError(e);
    }
  }

  const stateIcon = (state: NodeSummary["state"]): string =>
    state === "connected"
      ? "●"
      : state === "connecting"
        ? "○"
        : state === "unauthorised"
          ? "✕"
          : state === "incompatible"
            ? "!"
            : "✕";

  load();
</script>

<section aria-label="Nodes" class="nodes">
  <h2>Nodes</h2>

  <table class="table">
    <thead>
      <tr><th scope="col">Name</th><th scope="col">URL</th><th scope="col">State</th><th scope="col">Actions</th></tr>
    </thead>
    <tbody>
      {#each nodes as n (n.id)}
        <tr>
          <td>{n.name} {n.is_local ? "(local)" : ""}</td>
          <td class="mono">{n.url}</td>
          <td>
            <span class="state" data-state={n.state}>
              <span aria-hidden="true">{stateIcon(n.state)}</span> {n.state}
            </span>
          </td>
          <td>
            {#if n.state !== "connected"}
              <button type="button" class="btn-secondary" onclick={() => retry(n.id)}>Retry</button>
            {/if}
            {#if !n.is_local}
              <button type="button" class="btn-secondary" onclick={() => remove(n.id)}>Remove</button>
            {:else}
              <span class="hint">daemon</span>
            {/if}
          </td>
        </tr>
      {/each}
    </tbody>
  </table>

  <form class="add" onsubmit={(e) => e.preventDefault()}>
    <h3>Add remote node</h3>
    <label>
      Name
      <input class="input" bind:value={newName} placeholder="Home server" />
    </label>
    <label>
      URL
      <input class="input mono" bind:value={newUrl} placeholder="http://192.168.1.10:8080" />
    </label>
    <label>
      Token
      <input class="input mono" type="password" autocomplete="off" bind:value={newToken} placeholder="Bearer token from minewrapd" />
    </label>
    {#if error}
      <p role="alert" class="error">{error}</p>
    {/if}
    <button type="button" class="btn-primary" onclick={add}>Add node</button>
    <p class="hint">
      Recommended: reach the daemon through an SSH port-forward:
      <code>ssh -L 8080:127.0.0.1:8080 user@host</code> — see docs/remote-access.md.
    </p>
  </form>
</section>

<style>
  .nodes {
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    padding: var(--space-lg);
    background: var(--color-background);
    color: var(--color-foreground);
    font-family: var(--font-sans);
    height: 100%;
    overflow-y: auto;
  }
  h2, h3 { font-family: var(--font-mono); font-size: 14px; margin: 0; }
  .table { width: 100%; border-collapse: collapse; font-size: 13px; }
  th, td { text-align: left; padding: var(--space-sm) var(--space-md); border-bottom: 1px solid var(--color-border); }
  .mono { font-family: var(--font-mono); font-size: 12px; }
  .state { font-weight: 600; }
  .state[data-state="connected"] { color: var(--color-state-online); }
  .state[data-state="unauthorised"], .state[data-state="incompatible"], .state[data-state="offline"] { color: var(--color-state-crashed); }
  .state[data-state="connecting"] { color: var(--color-state-starting); }
  .add { display: flex; flex-direction: column; gap: var(--space-sm); border: 1px solid var(--color-border); border-radius: var(--radius-sm); padding: var(--space-md); }
  .add label { display: flex; flex-direction: column; gap: var(--space-xs); font-size: 13px; }
  .input {
    background: var(--color-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-foreground);
    padding: var(--space-sm) var(--space-md);
  }
  .btn-primary {
    background: var(--color-accent);
    color: var(--color-on-accent);
    border: none;
    border-radius: var(--radius-sm);
    padding: var(--space-sm) var(--space-lg);
    font-weight: 600;
    cursor: pointer;
  }
  .btn-secondary {
    background: transparent;
    color: var(--color-foreground);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: var(--space-xs) var(--space-md);
    font-weight: 600;
    cursor: pointer;
  }
  .hint { color: var(--color-muted-foreground); font-size: 12px; }
  .error { color: var(--color-destructive); font-size: 12px; }
  code { font-family: var(--font-mono); font-size: 12px; }
</style>
