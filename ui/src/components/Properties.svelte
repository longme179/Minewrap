<!-- Properties editor: typed fields + raw view, unsaved guard, inline
  errors. The doc is the Java properties round-trip `PropertiesDocument`;
  unknown keys pass through untouched. -->
<script lang="ts">
  import { createAdapter } from "../ipc/factory.js";

  interface Props {
    nodeId?: string;
    instanceId?: string;
  }

  let { nodeId = "local", instanceId = "survival" }: Props = $props();

  const ipc = createAdapter();

  let text = $state("");
  let savedText = $state("");
  let errors = $state<string[]>([]);
  let dirty = $state(false);
  let guardOpen = $state(false);

  async function load() {
    const doc = await ipc.readProperties(nodeId, instanceId);
    text = doc.text;
    savedText = doc.text;
    errors = doc.errors;
    dirty = false;
  }

  function edit(v: string) {
    text = v;
    dirty = v !== savedText;
  }

  async function save() {
    try {
      await ipc.writeProperties(nodeId, instanceId, { text, errors: [] });
      savedText = text;
      dirty = false;
      errors = [];
    } catch (e) {
      errors = [String(e)];
    }
  }

  function requestClose() {
    if (dirty) {
      guardOpen = true;
    }
  }

  function discard() {
    text = savedText;
    dirty = false;
    guardOpen = false;
  }

  // Initial load
  load();
</script>

<section aria-label="server.properties editor" class="props">
  <header class="head">
    <h2>server.properties</h2>
    <span role="status" class="dirty">{dirty ? "Unsaved changes" : "Saved"}</span>
    <span class="spacer"></span>
    <button type="button" class="btn-secondary" onclick={load}>Reload</button>
    <button type="button" class="btn-primary" onclick={save} disabled={!dirty}>Save</button>
  </header>

  {#if errors.length > 0}
    <div role="alert" class="error-summary">
      <p>Could not save:</p>
      <ul>
        {#each errors as err (err)}
          <li>{err}</li>
        {/each}
      </ul>
    </div>
  {/if}

  <label class="raw-label" for="props-raw">Raw properties (comments and order preserved)</label>
  <textarea
    id="props-raw"
    class="raw mono"
    value={text}
    oninput={(e) => edit((e.currentTarget as HTMLTextAreaElement).value)}
    rows="18"
    aria-describedby="props-hint"
    spellcheck="false"
  ></textarea>
  <p id="props-hint" class="hint">Typed fields land in Phase 5; the raw view is the source of truth for now.</p>

  {#if guardOpen}
    <div role="alertdialog" aria-labelledby="guard-title" aria-describedby="guard-desc" class="guard">
      <h3 id="guard-title">Unsaved changes</h3>
      <p id="guard-desc">You have unsaved changes to server.properties. Discard them?</p>
      <div class="guard-actions">
        <button type="button" class="btn-primary" onclick={discard}>Discard changes</button>
        <button type="button" class="btn-secondary" onclick={() => (guardOpen = false)}>Keep editing</button>
      </div>
    </div>
  {/if}

  <button type="button" class="btn-secondary ask-close" onclick={requestClose} hidden>Close editor</button>
</section>

<style>
  .props {
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
  .dirty { color: var(--color-muted-foreground); font-size: 12px; }
  .spacer { flex: 1; }
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
  .btn-secondary {
    background: transparent;
    color: var(--color-foreground);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: var(--space-sm) var(--space-lg);
    font-weight: 600;
    cursor: pointer;
  }
  .raw-label { font-weight: 600; font-size: 13px; }
  .raw {
    flex: 1;
    background: var(--color-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-foreground);
    padding: var(--space-md);
    font-size: 12px;
    line-height: 1.5;
    resize: vertical;
  }
  .mono { font-family: var(--font-mono); }
  .hint { color: var(--color-muted-foreground); font-size: 12px; margin: 0; }
  .error-summary {
    background: #2a1416;
    border: 1px solid var(--color-destructive);
    border-radius: var(--radius-sm);
    padding: var(--space-md);
  }
  .guard {
    background: var(--color-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: var(--space-lg);
  }
  .guard-actions { display: flex; gap: var(--space-sm); margin-top: var(--space-md); }
  .ask-close { display: none; }
</style>
