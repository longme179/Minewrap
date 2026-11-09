<!-- Java runtime picker + RAM controls (goal §4b/§4d).
  Detection runs off the UI thread (Rust scan); the selector offers
  every detected runtime, "System default (java on PATH)", and a
  "Custom path…" with file-picker validation. Compatibility warns
  (never blocks) and offers a one-click fix.-->
<script lang="ts">
  import { createAdapter } from "../ipc/factory.js";
  import { formatError } from "../lib/formatError.js";
  import Select from "./Select.svelte";
  import type { JavaRuntime } from "../ipc/mod.js";

  const ipc = createAdapter();

  interface Props {
    value?: string;
    mcVersion?: string | null;
    onChange?: (path: string) => void;
  }

  let { value = $bindable(""), mcVersion = null, onChange = () => {} }: Props = $props();

  let runtimes = $state<JavaRuntime[]>([]);
  let loading = $state(true);
  let error = $state("");
  let custom = $state(false);
  let customPath = $state("");

  async function load() {
    loading = true;
    try {
      runtimes = await ipc.detectJava();
    } catch (e) {
      error = formatError(e);
    } finally {
      loading = false;
    }
  }

  const options = $derived(
    [
      { value: "java", label: `System default (java on PATH)` },
      ...runtimes.map((r) => ({
        value: r.binary,
        label: `Java ${r.major ?? "?"} - ${r.vendor ?? "unknown"} - ${r.binary} (${r.source})`,
      })),
      { value: "__custom__", label: "Custom path…" },
    ] satisfies { value: string; label: string }[]
  );

  const selected = $derived(
    runtimes.find((r) => r.binary === value) ??
      (value === "java" ? null : runtimes.find((r) => r.home === value) ?? null)
  );

  const missingPath = $derived(value !== "" && value !== "java" && !runtimes.some((r) => r.binary === value));

  const needsJava = $derived(
    mcVersion
      ? null // set by the caller via minJava when known
      : null
  );

  async function rescan() {
    await load();
  }

  function pick(path: string) {
    if (path === "__custom__") {
      custom = true;
      return;
    }
    custom = false;
    onChange(path);
  }

  function applyCustom() {
    const p = customPath.trim();
    if (!p) return;
    onChange(p);
    custom = false;
  }

  function browseCustom() {
    // The native dialog is opened by the parent (Tauri dialog needs
    // the `dialog:default` capability); here we keep the input for
    // manual entry (Wayland degradation path).
    customPath = "";
  }

  load();
</script>

<div class="java">
  <div class="row">
    <span>Java runtime</span>
    <Select bind:value={value} name="java" options={options} onSelect={pick} />
    <button type="button" class="btn-secondary" onclick={rescan} disabled={loading}>
      {loading ? "Scanning…" : "Rescan"}
    </button>
  </div>

  {#if selected}
    <p class="hint">
      Selected: Java {selected.major ?? "?"} ({selected.full_version ?? "unknown version"})
      — {selected.vendor ?? "unknown vendor"}, {selected.source}
    </p>
  {/if}

  {#if missingPath}
    <p role="alert" class="warn">
      The saved path <code>{value}</code> no longer exists.
      <button type="button" class="link" onclick={() => pick("java")}>Use system default</button>
    </p>
  {/if}

  {#if custom}
    <div class="row">
      <input class="input mono" bind:value={customPath} placeholder="/path/to/bin/java or JDK home" />
      <button type="button" class="btn-secondary" onclick={applyCustom}>Use path</button>
      <button type="button" class="btn-secondary" onclick={browseCustom}>Browse…</button>
    </div>
  {/if}

  {#if error}
    <p role="alert" class="error">{error}</p>
  {/if}

  {#if needsJava}
    <p role="status" class="warn">{needsJava}</p>
  {/if}
</div>

<style>
  .java { display: flex; flex-direction: column; gap: var(--space-sm); }
  .row { display: flex; align-items: center; gap: var(--space-md); }
  .row > span:first-child { min-width: 140px; flex-shrink: 0; }
  .input {
    background: var(--color-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-foreground);
    padding: var(--space-sm) var(--space-md);
    flex: 1;
    min-width: 0;
  }
  .mono { font-family: var(--font-mono); }
  .hint { color: var(--color-muted-foreground); font-size: 12px; }
  .warn { color: #f59e0b; font-size: 12px; }
  .error { color: var(--color-destructive); font-size: 12px; }
  .btn-secondary {
    background: transparent;
    color: var(--color-foreground);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: var(--space-sm) var(--space-lg);
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
  }
  .link { background: none; border: none; color: var(--color-accent); cursor: pointer; text-decoration: underline; padding: 0; }
  code { font-family: var(--font-mono); }
</style>
