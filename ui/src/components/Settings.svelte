<!-- Instance settings: launch (java/JVM incl. Aikar's preset),
  probe, restart, with inline errors and a summary. Webhook is
  write-only (only `webhook_set` is shown). -->
<script lang="ts">
  import { createAdapter } from "../ipc/factory.js";
  import { formatError } from "../lib/formatError.js";
  import Select from "./Select.svelte";
  import JavaPicker from "./JavaPicker.svelte";
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
  let loadedFor = $state("");
  let savedSnapshot = $state("");

  let name = $state("");
  let workDir = $state("");
  let javaPath = $state("java");
  let minRamMb = $state(2048);
  let maxRamMb = $state(4096);
  let preset = $state<"none" | "aikar" | "custom">("custom");
  let jvmArgsText = $state("");
  let launchMode = $state<"jar" | "args-file" | "custom">("jar");
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

  function snapshot(): string {
    return JSON.stringify({
      name, workDir, javaPath, minRamMb, maxRamMb, preset, jvmArgsText,
      launchMode, launchArgsText, autostart, probe, restart,
    });
  }

  const dirty = $derived(loadedFor !== "" && snapshot() !== savedSnapshot);

  async function load(force = false) {
    const key = `${nodeId}:${instanceId}`;
    if (!force && key === loadedFor && !dirty) return;
    if (!force && dirty && loadedFor !== "" && key !== loadedFor) {
      if (typeof window !== "undefined" && !window.confirm("Discard unsaved settings changes?")) {
        return;
      }
    }
    loading = true;
    error = "";
    status = "";
    try {
      view = await ipc.getConfig(nodeId, instanceId);
      name = view.name;
      workDir = view.work_dir;
      javaPath = view.java_path || "java";
      minRamMb = parseRam(view.jvm_args, "-Xms", 2048);
      maxRamMb = parseRam(view.jvm_args, "-Xmx", 4096);
      preset = jvmArgsText_looksAikar(view.jvm_args) ? "aikar" : (view.jvm_args.join(" ").trim() ? "custom" : "none");
      jvmArgsText = view.jvm_args.join("\n");
      launchArgsText = view.launch_args.join(" ");
      autostart = view.autostart;
      probe = { ...view.probe };
      restart = { ...view.restart };
      loadedFor = key;
      savedSnapshot = snapshot();
    } catch (e) {
      error = formatError(e);
    } finally {
      loading = false;
    }
  }

  function revert() {
    if (savedSnapshot) {
      void load(true);
    }
  }

  async function save() {
    error = "";
    status = "";
    saving = true;
    if (maxRamMb < minRamMb) {
      error = "Max RAM must be >= Min RAM.";
      saving = false;
      return;
    }
    try {
      await ipc.updateConfig(nodeId, instanceId, {
        name: name.trim() || undefined,
        work_dir: workDir.trim() || undefined,
        java_path: javaPath.trim() || undefined,
        jvm_args: finalJvmArgs(),
        launch_args: launchMode === "custom" ? (launchArgsText.trim() ? launchArgsText.trim().split(/\s+/) : undefined) : launchArgsText.trim() ? launchArgsText.trim().split(/\s+/) : undefined,
        autostart,
        probe,
        restart,
      });
      savedSnapshot = snapshot();
      loadedFor = `${nodeId}:${instanceId}`;
      status = "Saved.";
      onSaved();
    } catch (e) {
      error = formatError(e);
    } finally {
      saving = false;
    }
  }

  // --- RAM / JVM args (goal §4d) ---

  function parseRam(args: string[], prefix: string, fallback: number): number {
    for (const a of args) {
      if (a.startsWith(prefix)) {
        const m = a.match(/^-(?:Xms|Xmx)(\d+)([MmGg])?$/);
        if (m) {
          const n = parseInt(m[1], 10);
          return m[2] && (m[2] === "G" || m[2] === "g") ? n * 1024 : n;
        }
      }
    }
    return fallback;
  }

  function ramArg(prefix: string, mb: number): string {
    if (mb >= 1024 && mb % 1024 === 0) return `${prefix}${mb / 1024}G`;
    return `${prefix}${mb}M`;
  }

  function jvmArgsText_looksAikar(args: string[]): boolean {
    return args.some((a) => a.includes("aikars.flags"));
  }

  const ramConflict = $derived(minRamMb > maxRamMb);

  function finalJvmArgs(): string[] {
    if (preset === "none") return [];
    if (preset === "aikar") {
      return [
        ramArg("-Xms", minRamMb),
        ramArg("-Xmx", maxRamMb),
        "-XX:+UseG1GC",
        "-XX:+ParallelRefProcEnabled",
        "-XX:MaxGCPauseMillis=200",
        "-XX:+UnlockExperimentalVMOptions",
        "-XX:+DisableExplicitGC",
        "-XX:+AlwaysPreTouch",
        "-XX:G1NewSizeRatio=30",
        "-XX:G1MaxNewSizeRatio=40",
        "-XX:G1HeapRegionSize=8M",
        "-XX:G1ReservePercent=20",
        "-XX:G1HeapWastePercent=5",
        "-XX:G1MixedGCCountTarget=4",
        "-XX:InitiatingHeapOccupancyPercent=15",
        "-XX:G1MixedGCLiveThresholdPercent=90",
        "-XX:G1RSetUpdatingPauseTimePercent=5",
        "-XX:SurvivorRatio=32",
        "-XX:+PerfDisableSharedMem",
        "-XX:MaxTenuringThreshold=1",
        "-Dusing.aikars.flags=https://mcflags.env",
        "-Daikars.flags.file=aikars.flags",
        "-Dlog4j2.formatMsgNoLookups=true",
      ];
    }
    const extra = jvmArgsText.split("\n").map((s) => s.trim()).filter(Boolean);
    return [ramArg("-Xms", minRamMb), ramArg("-Xmx", maxRamMb), ...extra];
  }

  function applyAikar() {
    preset = "aikar";
  }

  $effect(() => {
    void load();
  });
</script>

<section aria-label="Settings" class="settings">
  <div class="head">
    <h2>Settings</h2>
    <span role="status" class="dirty">{dirty ? "Unsaved changes" : "Saved"}</span>
  </div>

  {#if loading}
    <p role="status" class="hint">Loading…</p>
  {:else}
    <form class="form" onsubmit={(e) => e.preventDefault()}>
      <fieldset>
        <legend>Launch</legend>
        <label class="row"><span>Display name</span> <input class="input" bind:value={name} /></label>
        <label class="row"><span>Work dir</span> <input class="input mono" bind:value={workDir} /></label>
        <JavaPicker bind:value={javaPath} />
        <!-- RAM / JVM args (goal §4d) -->
        <div class="row">
          <span>Min RAM (MiB)</span>
          <input class="input" type="number" min="256" max="65536" bind:value={minRamMb} />
        </div>
        <div class="row">
          <span>Max RAM (MiB)</span>
          <input class="input" type="number" min="256" max="65536" bind:value={maxRamMb} />
        </div>
        {#if ramConflict}
          <p role="alert" class="error">Max RAM must be >= Min RAM.</p>
        {/if}
        <div class="row">
          <span>Preset</span>
          <Select
            bind:value={preset}
            name="jvm-preset"
            options={[
              { value: "none", label: "None" },
              { value: "aikar", label: "Aikar's flags" },
              { value: "custom", label: "Custom" },
            ]}
          />
        </div>
        {#if preset === "custom"}
          <label class="row col">
            <span>Extra JVM args (one per line)</span>
            <textarea class="input mono" rows="4" bind:value={jvmArgsText} spellcheck="false"></textarea>
          </label>
        {/if}
        <p class="hint">Final: <code>{finalJvmArgs().join(" ")}</code></p>
        <!-- Launch mode (goal §3) -->
        <div class="row">
          <span>Launch mode</span>
          <Select
            bind:value={launchMode}
            name="launch-mode"
            options={[
              { value: "jar", label: "Jar (-jar server.jar nogui)" },
              { value: "args-file", label: "Args file (@user_jvm_args.txt…)" },
              { value: "custom", label: "Custom" },
            ]}
          />
        </div>
        <label class="row"><span>Launch args</span> <input class="input mono" bind:value={launchArgsText} /></label>
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
        <button type="button" class="btn-primary" onclick={save} disabled={saving || !dirty}>
          {saving ? "Saving…" : "Save"}
        </button>
        <button type="button" class="btn-secondary" onclick={revert} disabled={saving || !dirty}>Revert</button>
        <span role="status" class="status">{dirty ? "Unsaved changes" : status}</span>
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
  .head { display: flex; align-items: center; gap: var(--space-md); }
  h2 { font-family: var(--font-mono); font-size: 14px; margin: 0; }
  .dirty { color: var(--color-muted-foreground); font-size: 12px; }
  .form { display: flex; flex-direction: column; gap: var(--space-lg); max-width: 640px; }
  fieldset { border: 1px solid var(--color-border); border-radius: var(--radius-sm); padding: var(--space-md); }
  legend { padding: 0 var(--space-sm); color: var(--color-muted-foreground); font-size: 12px; }
  .row { display: flex; align-items: center; gap: var(--space-md); margin-bottom: var(--space-sm); font-size: 13px; }
  .row.col { align-items: flex-start; flex-direction: column; }
  .row > span:first-child { min-width: 140px; flex-shrink: 0; }
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
