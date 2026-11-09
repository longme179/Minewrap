<!-- Add / remove instances (local node). Instance IDs must match
  ^[a-z0-9][a-z0-9_-]{0,62}[a-z0-9]$ (v1 parity). -->
<script lang="ts">
  import { createAdapter } from "../ipc/factory.js";
  import { formatError } from "../lib/formatError.js";

  const ipc = createAdapter();

  interface Props {
    onChanged?: () => void;
  }

  let { onChanged = () => {} }: Props = $props();

  const ID_RE = /^[a-z0-9][a-z0-9_-]{0,62}[a-z0-9]$/;

  let id = $state("");
  let name = $state("");
  let workDir = $state("");
  let javaPath = $state("java");
  let jvmArgs = $state("-Xms2G -Xmx4G");
  let error = $state("");
  let status = $state("");
  let busy = $state(false);

  async function add() {
    error = "";
    status = "";
    const trimmedId = id.trim();
    const trimmedDir = workDir.trim();
    if (!ID_RE.test(trimmedId)) {
      error = "ID must be lowercase letters/digits/-/_ 1–64 chars, and not start/end with - or _.";
      return;
    }
    if (!trimmedDir) {
      error = "Work dir is required.";
      return;
    }
    busy = true;
    try {
      await ipc.addInstance({
        id: trimmedId,
        name: name.trim() || undefined,
        work_dir: trimmedDir,
        java_path: javaPath.trim() || undefined,
        jvm_args: jvmArgs.trim() ? jvmArgs.trim().split(/\s+/) : undefined,
      });
      status = `Added '${trimmedId}'.`;
      id = "";
      name = "";
      workDir = "";
      onChanged();
    } catch (e) {
      error = formatError(e);
    } finally {
      busy = false;
    }
  }
</script>

<section aria-label="Add instance" class="add">
  <h2>Add instance</h2>
  <form class="form" onsubmit={(e) => e.preventDefault()}>
    <label>
      ID (lowercase, no spaces)
      <input class="input mono" bind:value={id} placeholder="survival" autocomplete="off" />
    </label>
    <label>
      Display name (optional)
      <input class="input" bind:value={name} placeholder="Survival" />
    </label>
    <label>
      Work dir
      <input class="input mono" bind:value={workDir} placeholder="/srv/minecraft/survival" />
    </label>
    <label>
      Java path
      <input class="input mono" bind:value={javaPath} />
    </label>
    <label>
      JVM args (space-separated)
      <input class="input mono" bind:value={jvmArgs} />
    </label>
    {#if error}
      <p role="alert" class="error">{error}</p>
    {/if}
    {#if status}
      <p role="status" class="status">{status}</p>
    {/if}
    <div class="actions">
      <button type="button" class="btn-primary" onclick={add} disabled={busy}>
        {busy ? "Adding…" : "Add instance"}
      </button>
    </div>
  </form>
</section>

<style>
  .add {
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
  h2 { font-family: var(--font-mono); font-size: 14px; margin: 0; }
  .form { display: flex; flex-direction: column; gap: var(--space-sm); max-width: 520px; }
  .form label { display: flex; flex-direction: column; gap: var(--space-xs); font-size: 13px; }
  .input {
    background: var(--color-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-foreground);
    padding: var(--space-sm) var(--space-md);
  }
  .mono { font-family: var(--font-mono); }
  .actions { display: flex; gap: var(--space-sm); margin-top: var(--space-sm); }
  .btn-primary {
    background: var(--color-accent);
    color: var(--color-on-accent);
    border: none;
    border-radius: var(--radius-sm);
    padding: var(--space-sm) var(--space-lg);
    font-weight: 600;
    cursor: pointer;
  }
  .btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }
  .error { color: var(--color-destructive); font-size: 12px; }
  .status { color: var(--color-muted-foreground); font-size: 12px; }
</style>
