<!-- Add instance wizard (goal §6): folder -> detect -> Java -> RAM ->
  review. Reuses Settings' RAM/preset logic; validates EULA before
  the instance is created. -->
<script lang="ts">
  import { createAdapter, isTauri } from "../ipc/factory.js";
  import { formatError } from "../lib/formatError.js";
  import Select from "./Select.svelte";
  import JavaPicker from "./JavaPicker.svelte";
  import type { JavaRuntime, LaunchCandidateDto, WorkDirReport } from "../ipc/mod.js";

  const ipc = createAdapter();

  interface Props {
    onChanged?: () => void;
  }

  let { onChanged = () => {} }: Props = $props();

  const ID_RE = /^[a-z0-9][a-z0-9_-]{0,62}[a-z0-9]$/;

  let step = $state(1);
  let id = $state("");
  let name = $state("");
  let workDir = $state("");
  let javaPath = $state("java");
  let minRam = $state(2048);
  let maxRam = $state(4096);
  let preset = $state<"none" | "aikar" | "custom">("custom");
  let launchMode = $state<"jar" | "args-file" | "custom">("jar");
  let launchArgs = $state("");
  let candidates = $state<LaunchCandidateDto[]>([]);
  let report = $state<WorkDirReport | null>(null);
  let scanError = $state("");
  let busy = $state(false);
  let error = $state("");
  let status = $state("");
  let runtimes = $state<JavaRuntime[]>([]);
  let scanNote = $state("");
  let browsing = $state(false);

  const detected = $derived(
    candidates.find((c) => c.kind === "jar") ?? candidates[0] ?? null
  );

  async function scan() {
    scanError = "";
    scanNote = "";
    report = null;
    if (!workDir.trim()) {
      scanNote = "Enter a work dir first.";
      return false;
    }
    try {
      const res = await ipc.scanLaunch("local", workDir.trim());
      candidates = res.candidates;
      report = res.report;
      const d = candidates.find((c) => c.kind === "jar") ?? candidates[0] ?? null;
      if (d?.suggested_launch_args?.length) {
        launchArgs = d.suggested_launch_args.join(" ");
      }
      if (d?.mc_version) {
        const need = d.min_java;
        const sel = runtimes.find((r) => r.binary === javaPath);
        if (need && sel?.major && sel.major < need) {
          scanNote = `Detected Minecraft ${d.mc_version} needs Java ${need}+; selected is ${sel.major}.`;
        } else {
          scanNote = `Detected: ${d.server_type ?? "Server"} ${d.mc_version}`;
        }
      } else if (res.report.notes.length) {
        scanNote = res.report.notes[0] ?? "";
      }
      return true;
    } catch (e) {
      scanError = formatError(e);
      return false;
    }
  }

  async function browseWorkDir() {
    browsing = true;
    scanError = "";
    try {
      if (isTauri()) {
        const { open } = await import("@tauri-apps/plugin-dialog");
        const picked = await open({ directory: true, multiple: false, title: "Select server folder" });
        if (typeof picked === "string" && picked) {
          workDir = picked;
          await scan();
        }
      } else {
        scanNote = "Folder picker is available in the desktop app. Enter the path manually.";
      }
    } catch (e) {
      scanError = formatError(e);
    } finally {
      browsing = false;
    }
  }

  async function loadRuntimes() {
    try {
      runtimes = await ipc.detectJava();
    } catch {
      // non-fatal
    }
  }

  async function next() {
    error = "";
    if (step === 1) {
      if (!ID_RE.test(id.trim())) {
        error = "ID must be lowercase letters/digits/-/_ 1–64 chars.";
        return;
      }
      if (!workDir.trim()) {
        error = "Work dir is required.";
        return;
      }
      const ok = await scan();
      if (!ok && scanError) return;
    }
    if (step === 3 && maxRam < minRam) {
      error = "Max RAM must be >= Min RAM.";
      return;
    }
    if (step < 4) {
      step += 1;
      return;
    }
    await create();
  }

  function back() {
    if (step > 1) step -= 1;
  }

  async function create() {
    busy = true;
    try {
      await ipc.addInstance({
        id: id.trim(),
        name: name.trim() || undefined,
        work_dir: workDir.trim(),
        java_path: javaPath.trim() || undefined,
        jvm_args: finalJvm(),
        launch_args: launchArgs.trim() ? launchArgs.trim().split(/\s+/) : undefined,
      });
      status = "Instance added.";
      onChanged();
    } catch (e) {
      error = formatError(e);
    } finally {
      busy = false;
    }
  }

  function ram(mb: number) {
    return mb >= 1024 && mb % 1024 === 0 ? `${mb / 1024}G` : `${mb}M`;
  }

  function finalJvm(): string[] {
    if (preset === "none") return [];
    if (preset === "aikar") return [`-Xms${ram(minRam)}`, `-Xmx${ram(maxRam)}`, "-XX:+UseG1GC", "-Dlog4j2.formatMsgNoLookups=true"];
    return [`-Xms${ram(minRam)}`, `-Xmx${ram(maxRam)}`];
  }

  loadRuntimes();
</script>

<section aria-label="Add instance wizard" class="wizard">
  <h2>Add instance — step {step}/4</h2>

  {#if status}
    <p role="status" class="status">{status}</p>
  {/if}
  {#if error}
    <p role="alert" class="error">{error}</p>
  {/if}

  {#if step === 1}
    <label>ID <input class="input mono" bind:value={id} placeholder="survival" /></label>
    <label>Display name (optional) <input class="input" bind:value={name} placeholder="Survival" /></label>
    <label>
      Work dir
      <div class="row">
        <input class="input mono" bind:value={workDir} placeholder="/srv/minecraft/survival" />
        <button type="button" class="btn-secondary" onclick={browseWorkDir} disabled={browsing}>
          {browsing ? "Opening…" : "Browse"}
        </button>
        <button type="button" class="btn-secondary" onclick={() => void scan()} disabled={browsing || !workDir.trim()}>
          Scan
        </button>
      </div>
    </label>
    {#if scanError}
      <p role="alert" class="error">{scanError}</p>
    {/if}
    {#if report}
      <ul class="cands">
        <li>Readable: {report.readable ? "yes" : "no"} · Writable: {report.writable ? "yes" : "no"}</li>
        <li>server.properties: {report.has_server_properties ? "found" : "missing"} · eula.txt: {report.has_eula ? "found" : "missing"}</li>
        {#each report.notes as note (note)}
          <li>{note}</li>
        {/each}
      </ul>
    {/if}
    {#if candidates.length > 0}
      <ul class="cands">
        {#each candidates as c (c.path)}
          <li>{c.suggested_launch_args.join(" ")}</li>
        {/each}
      </ul>
    {/if}
    {#if scanNote}
      <p role="status" class="status">{scanNote}</p>
    {/if}
  {:else if step === 2}
    <JavaPicker bind:value={javaPath} />
    {#if detected?.mc_version}
      <p class="hint">
        Detected: {detected.server_type ?? "Server"} {detected.mc_version}
        {#if detected.min_java}— needs Java {detected.min_java}+{/if}
      </p>
    {/if}
  {:else if step === 3}
    <div class="row"><span>Min RAM (MiB)</span> <input class="input" type="number" min="256" max="65536" bind:value={minRam} /></div>
    <div class="row"><span>Max RAM (MiB)</span> <input class="input" type="number" min="256" max="65536" bind:value={maxRam} /></div>
    <div class="row">
      <span>Preset</span>
      <Select
        bind:value={preset}
        name="preset"
        options={[
          { value: "none", label: "None" },
          { value: "aikar", label: "Aikar's flags" },
          { value: "custom", label: "Custom" },
        ]}
      />
    </div>
  {:else}
    <p class="hint">Review: {id} → <code>{finalJvm().join(" ")}</code></p>
    <div class="row">
      <span>Launch mode</span>
      <Select
        bind:value={launchMode}
        name="mode"
        options={[
          { value: "jar", label: "Jar" },
          { value: "args-file", label: "Args file" },
          { value: "custom", label: "Custom" },
        ]}
      />
    </div>
    <label>Launch args <input class="input mono" bind:value={launchArgs} /></label>
  {/if}

  <div class="actions">
    {#if step > 1}
      <button type="button" class="btn-secondary" onclick={back}>Back</button>
    {/if}
    <button type="button" class="btn-primary" onclick={next} disabled={busy}>
      {step === 4 ? (busy ? "Adding…" : "Add instance") : "Next"}
    </button>
  </div>
</section>

<style>
  .wizard { display: flex; flex-direction: column; gap: var(--space-md); padding: var(--space-lg); background: var(--color-background); color: var(--color-foreground); font-family: var(--font-sans); height: 100%; overflow-y: auto; }
  h2 { font-family: var(--font-mono); font-size: 14px; margin: 0; }
  .row { display: flex; align-items: center; gap: var(--space-md); }
  .input { background: var(--color-card); border: 1px solid var(--color-border); border-radius: var(--radius-sm); color: var(--color-foreground); padding: var(--space-sm) var(--space-md); flex: 1; min-width: 0; }
  .mono { font-family: var(--font-mono); }
  .cands { color: var(--color-muted-foreground); font-size: 12px; font-family: var(--font-mono); }
  .actions { display: flex; gap: var(--space-sm); margin-top: var(--space-sm); }
  .btn-primary { background: var(--color-accent); color: var(--color-on-accent); border: none; border-radius: var(--radius-sm); padding: var(--space-sm) var(--space-lg); font-weight: 600; cursor: pointer; }
  .btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }
  .btn-secondary { background: transparent; color: var(--color-foreground); border: 1px solid var(--color-border); border-radius: var(--radius-sm); padding: var(--space-sm) var(--space-lg); font-weight: 600; cursor: pointer; }
  .error { color: var(--color-destructive); font-size: 12px; }
  .status { color: var(--color-muted-foreground); font-size: 12px; }
  .hint { color: var(--color-muted-foreground); font-size: 12px; }
  label { display: flex; flex-direction: column; gap: var(--space-xs); font-size: 13px; }
</style>
