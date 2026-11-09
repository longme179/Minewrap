<!-- Instance settings: launch (java/JVM incl. Aikar's preset),
  probe, restart, with inline errors and a summary. Webhook is
  write-only (only `webhook_set` is shown). -->
<script lang="ts">
  import { createAdapter } from "../ipc/factory.js";
  import { formatError } from "../lib/formatError.js";
  import Select from "./Select.svelte";
  import type { ConfigView } from "../ipc/mod.js";

  const ipc = createAdapter();

  interface Props {
    nodeId?: string;
    instanceId?: string;
    view?: ConfigView | null;
    onSaved?: () => void;
  }

  let { nodeId = "local", instanceId = "survival", view = $bindable(null), onSaved = () => {} }: Props = $props();

  let loading = $state(true);
  let error = $state("");
  let status = $state("");
  let saving = $state(false);

  let name = $state("");
  let workDir = $state("");
  let javaPath = $state("");
  let jvmArgsText = $state("");
  let launchArgsText = $state("");
  let autostart = $state(false);
  let probe = $state({ host: "127.0.0.1", port: 25565, query_enabled: false, timeout: 3.0 });
  let restart = $state({
    policy: "on-crash",
    max_retries: 5,
    backoff_base_seconds: 5.0,
    backoff_max_seconds: 300.0,
    reset_after_online_seconds: 600.0,
  });

  async function load() {
    loading = true;
    try {
      view = await ipc.getConfig(nodeId, instanceId);
      name = view.name;
      workDir = view.work_dir;
      javaPath = view.java_path;
      jvmArgsText = view.jvm_args.join(" ");
      launchArgsText = view.launch_args.join(" ");
      autostart = view.autostart;
      probe = { ...view.probe };
      restart = { ...view.restart };
    } catch (e) {
      error = formatError(e);
    } finally {
      loading = false;
    }
  }

  async function save() {
    error = "";
    status = "";
    saving = true;
    try {
      await ipc.updateConfig(nodeId, instanceId, {
        name: name.trim() || undefined,
        work_dir: workDir.trim() || undefined,
        java_path: javaPath.trim() || undefined,
        jvm_args: jvmArgsText.trim() ? jvmArgsText.trim().split(/\s+/) : [],
        launch_args: launchArgsText.trim() ? launchArgsText.trim().split(/\s+/) : undefined,
        autostart,
        probe,
        restart,
      });
      status = "Saved.";
      onSaved();
    } catch (e) {
      error = formatError(e);
    } finally {
      saving = false;
    }
  }

  function applyAikar() {
    jvmArgsText =
      "-Xms2G -Xmx4G -XX:+UseG1GC -XX:+ParallelRefProcEnabled -XX:MaxGCPauseMillis=200 " +
      "-XX:+UnlockExperimentalVMOptions -XX:+DisableExplicitGC -XX:+AlwaysPreTouch " +
      "-XX:G1NewSizeRatio=30 -XX:G1MaxNewSizeRatio=40 -XX:G1HeapRegionSize=8M " +
      "-XX:G1ReservePercent=20 -XX:G1HeapWastePercent=5 -XX:G1MixedGCCountTarget=4 " +
      "-XX:InitiatingHeapOccupancyPercent=15 -XX:G1MixedGCLiveThresholdPercent=90 " +
      "-XX:G1RSetUpdatingPauseTimePercent=5 -XX:SurvivorRatio=32 " +
      "-XX:+PerfDisableSharedMem -XX:MaxTenuringThreshold=1 " +
      "-Dusing.aikars.flags=https://mcflags.env -Daikars.flags.file=aikars.flags " +
      "-Dlog4j2.formatMsgNoLookups=true";
  }

  $effect(() => {
    if (view === null) {
      void load();
    }
  });
</script>

<section aria-label="Settings" class="settings">
  <h2>Settings</h2>

  {#if loading}
    <p role="status" class="hint">Loading…</p>
  {:else}
    <form class="form" onsubmit={(e) => e.preventDefault()}>
      <fieldset>
        <legend>Launch</legend>
        <label class="row">Display name <input class="input" bind:value={name} /></label>
        <label class="row">Work dir <input class="input mono" bind:value={workDir} /></label>
        <label class="row">Java path <input class="input mono" bind:value={javaPath} /></label>
        <label class="row">JVM args <input class="input mono" bind:value={jvmArgsText} /></label>
        <label class="row">Launch args <input class="input mono" bind:value={launchArgsText} /></label>
        <label class="row"><input type="checkbox" bind:checked={autostart} /> Auto-start</label>
        <div class="actions">
          <button type="button" class="btn-secondary" onclick={applyAikar}>Apply Aikar's Flags</button>
        </div>
      </fieldset>

      <fieldset>
        <legend>Probe</legend>
        <label class="row">Host <input class="input mono" bind:value={probe.host} /></label>
        <label class="row">Port <input class="input" type="number" min="1" max="65535" bind:value={probe.port} /></label>
        <label class="row">Timeout (s) <input class="input" type="number" min="0.1" step="0.1" bind:value={probe.timeout} /></label>
        <label class="row"><input type="checkbox" bind:checked={probe.query_enabled} /> Query enabled</label>
      </fieldset>

      <fieldset>
        <legend>Restart</legend>
        <div class="row">
          <span>Policy</span>
          <Select
            bind:value={restart.policy}
            name="policy"
            options={[
              { value: "never", label: "never" },
              { value: "on-crash", label: "on-crash" },
            ]}
          />
        </div>
        <label class="row">Max retries <input class="input" type="number" min="0" max="100" bind:value={restart.max_retries} /></label>
        <label class="row">Backoff base (s) <input class="input" type="number" min="0.1" step="0.1" bind:value={restart.backoff_base_seconds} /></label>
        <label class="row">Backoff max (s) <input class="input" type="number" min="0.1" step="0.1" bind:value={restart.backoff_max_seconds} /></label>
        <label class="row">Reset after online (s) <input class="input" type="number" min="0" step="1" bind:value={restart.reset_after_online_seconds} /></label>
      </fieldset>

      {#if error}
        <p role="alert" class="error">{error}</p>
      {/if}
      <div class="actions">
        <button type="button" class="btn-primary" onclick={save} disabled={saving}>
          {saving ? "Saving…" : "Save"}
        </button>
        <span role="status" class="status">{status}</span>
      </div>
    </form>
  {/if}
</section>

<style>
  .settings {
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
  .form { display: flex; flex-direction: column; gap: var(--space-lg); max-width: 640px; }
  fieldset { border: 1px solid var(--color-border); border-radius: var(--radius-sm); padding: var(--space-md); }
  legend { padding: 0 var(--space-sm); color: var(--color-muted-foreground); font-size: 12px; }
  .row { display: flex; align-items: center; gap: var(--space-md); margin-bottom: var(--space-sm); font-size: 13px; }
  .input {
    background: var(--color-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-foreground);
    padding: var(--space-sm) var(--space-md);
    flex: 1;
  }
  .mono { font-family: var(--font-mono); }
  .actions { display: flex; align-items: center; gap: var(--space-sm); margin-top: var(--space-sm); }
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
  .hint { color: var(--color-muted-foreground); font-size: 12px; }
  .error { color: var(--color-destructive); font-size: 12px; }
  .status { color: var(--color-muted-foreground); font-size: 12px; }
</style>
