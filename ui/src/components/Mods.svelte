<!-- Mods manager: list / search-filter / toggle `.jar <-> .jar.disabled`.
  Disabling requires confirmation; enabling is immediate. -->
<script lang="ts">
  import { MockIpcAdapter } from "../ipc/mock.js";
  import type { ModDto } from "../ipc/mod.js";

  interface Props {
    nodeId?: string;
    instanceId?: string;
  }

  let { nodeId = "local", instanceId = "survival" }: Props = $props();

  const ipc = new MockIpcAdapter();

  let mods = $state<ModDto[]>([]);
  let filter = $state("");
  let confirming = $state<string | null>(null);
  let status = $state("");

  async function load() {
    void nodeId;
    void instanceId;
    mods = await ipc.listMods();
  }

  const filtered = $derived(
    filter ? mods.filter((m) => m.name.toLowerCase().includes(filter.toLowerCase())) : mods
  );

  async function toggle(m: ModDto) {
    if (!m.enabled && confirming !== m.name) {
      confirming = m.name;
      return;
    }
    confirming = null;
    try {
      void nodeId;
      void instanceId;
      await ipc.toggleMod();
      status = `${m.name} ${m.enabled ? "disabled" : "enabled"}.`;
      mods = mods.map((x) =>
        x.name === m.name ? { ...x, enabled: !m.enabled } : x
      );
    } catch (e) {
      status = String(e);
    }
  }

  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    const units = ["KiB", "MiB", "GiB"];
    let v = bytes / 1024;
    let i = 0;
    while (v >= 1024 && i < units.length - 1) {
      v /= 1024;
      i += 1;
    }
    return `${v.toFixed(1)} ${units[i]}`;
  }

  load();
</script>

<section aria-label="Mods" class="mods">
  <header class="head">
    <h2>Mods</h2>
    <label>
      <span class="sr-only">Search mods</span>
      <input
        class="input"
        type="search"
        bind:value={filter}
        placeholder="Search mods…"
        aria-label="Search mods"
      />
    </label>
    <span class="spacer"></span>
    <span role="status" class="status">{status}</span>
  </header>

  {#if filtered.length === 0}
    <p class="empty">{filter ? "No results — try clearing the filter." : "No mods found."}</p>
  {:else}
    <table class="table">
      <thead>
        <tr><th scope="col">Name</th><th scope="col">Size</th><th scope="col">State</th><th scope="col">Action</th></tr>
      </thead>
      <tbody>
        {#each filtered as m (m.name)}
          <tr>
            <td class="mono">{m.name}</td>
            <td>{formatSize(m.size)}</td>
            <td>{m.enabled ? "enabled" : "disabled"}</td>
            <td>
              <button type="button" class="btn-secondary" onclick={() => toggle(m)}>
                {confirming === m.name ? "Confirm disable" : m.enabled ? "Disable" : "Enable"}
              </button>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</section>

<style>
  .mods {
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    padding: var(--space-lg);
    background: var(--color-background);
    color: var(--color-foreground);
    font-family: var(--font-sans);
    height: 100%;
  }
  .head { display: flex; align-items: center; gap: var(--space-md); }
  h2 { font-family: var(--font-mono); font-size: 14px; margin: 0; }
  .input {
    background: var(--color-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-foreground);
    padding: var(--space-sm) var(--space-md);
  }
  .spacer { flex: 1; }
  .status { color: var(--color-muted-foreground); font-size: 12px; }
  .table { width: 100%; border-collapse: collapse; font-size: 13px; }
  th, td { text-align: left; padding: var(--space-sm) var(--space-md); border-bottom: 1px solid var(--color-border); }
  .mono { font-family: var(--font-mono); font-size: 12px; }
  .btn-secondary {
    background: transparent;
    color: var(--color-foreground);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: var(--space-xs) var(--space-md);
    font-weight: 600;
    cursor: pointer;
  }
  .empty { color: var(--color-muted-foreground); }
  .sr-only {
    position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px;
    overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; border: 0;
  }
</style>
