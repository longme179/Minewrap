<!-- Notifications (Discord): enabled / mode / interval / log-lines /
  redaction, send-test result, rate-limit status. Webhook is write-only —
  only `webhook_set` is shown, never the URL. Settings persist via the
  IPC `update_config` command (config 0600). -->
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

  let enabled = $state(true);
  let mode = $state("edit-in-place");
  let interval = $state(600);
  let logLines = $state(15);
  let maxLogChars = $state(1500);
  let firstPush = $state(10);
  let stateChangePosts = $state(true);
  let debounce = $state(30);
  let redact = $state("");
  let webhookSet = $state(false);
  let webhook = $state("");
  let result = $state("");
  let sending = $state(false);
  let saving = $state(false);
  let loaded = $state(false);

  let loadError = $state("");

  async function load() {
    try {
      loadError = "";
      view = await ipc.getConfig(nodeId, instanceId);
      enabled = view.discord.enabled;
      mode = view.discord.mode;
      interval = view.discord.interval_seconds;
      logLines = view.discord.log_lines;
      maxLogChars = view.discord.max_log_chars;
      firstPush = view.discord.first_push_delay_seconds;
      stateChangePosts = view.discord.state_change_posts;
      debounce = view.discord.state_change_debounce_seconds;
      redact = (view.discord.redact_regexes ?? []).join("\n");
      webhookSet = view.discord.webhook_set;
      loaded = true;
    } catch (e) {
      loadError = formatError(e);
      result = formatError(e);
    }
  }

  async function save() {
    saving = true;
    try {
      const discord: Record<string, unknown> = {
        enabled,
        mode,
        interval_seconds: interval,
        log_lines: logLines,
        max_log_chars: maxLogChars,
        first_push_delay_seconds: firstPush,
        state_change_posts: stateChangePosts,
        state_change_debounce_seconds: debounce,
        redact_regexes: redact.split("\n").map((s) => s.trim()).filter(Boolean),
      };
      if (webhook.trim()) discord.webhook_url = webhook.trim();
      await ipc.updateConfig(nodeId, instanceId, { discord });
      if (webhook.trim()) {
        webhook = "";
        webhookSet = true;
      }
      result = "Saved.";
      onSaved();
    } catch (e) {
      result = formatError(e);
    } finally {
      saving = false;
    }
  }

  async function test() {
    sending = true;
    try {
      const res = await ipc.discordTest(nodeId, instanceId);
      result = res.ok ? "Test message sent." : `Failed: ${res.error ?? "unknown"}`;
    } catch (e) {
      result = formatError(e);
    } finally {
      sending = false;
    }
  }

  let effectKey = $state("");
  $effect(() => {
    const key = `${nodeId}:${instanceId}`;
    if (key !== effectKey) {
      effectKey = key;
      loaded = false;
      view = null;
      void load();
    }
  });
</script>

<section aria-label="Discord notifications" class="discord">
  <h2>Discord</h2>

  {#if !loaded && !loadError}
    <p role="status" class="hint">Loading…</p>
  {:else if loadError}
    <p role="alert" class="error">{loadError}</p>
    <button type="button" class="btn-secondary" onclick={load}>Retry</button>
  {:else}
    <form class="form" onsubmit={(e) => e.preventDefault()}>
      <fieldset>
        <legend>Notifier</legend>
        <label class="row">
          <input type="checkbox" bind:checked={enabled} />
          Enabled
        </label>
        <div class="row">
          <span>Mode</span>
          <Select
            bind:value={mode}
            name="mode"
            options={[
              { value: "edit-in-place", label: "Edit in place (one message, updated)" },
              { value: "new-message", label: "New message every interval" },
            ]}
          />
        </div>
        <label class="row">
          Interval (seconds)
          <input class="input" type="number" min="10" bind:value={interval} />
        </label>
        <label class="row">
          First push delay (s)
          <input class="input" type="number" min="0" bind:value={firstPush} />
        </label>
        <label class="row">
          Log lines per message
          <input class="input" type="number" min="0" max="50" bind:value={logLines} />
        </label>
        <label class="row">
          Max log chars
          <input class="input" type="number" min="0" max="4096" bind:value={maxLogChars} />
        </label>
        <label class="row">
          <input type="checkbox" bind:checked={stateChangePosts} />
          State-change posts
        </label>
        <label class="row">
          State-change debounce (s)
          <input class="input" type="number" min="0" bind:value={debounce} />
        </label>
        <label class="row col">
          Redact regexes (one per line)
          <textarea class="input mono" rows="3" bind:value={redact} spellcheck="false"></textarea>
        </label>
      </fieldset>

      <fieldset>
        <legend>Webhook</legend>
        <p class="hint">
          {webhookSet ? "A webhook URL is stored in the keyring. Enter a new one to replace it." : "No webhook stored."}
        </p>
        <label class="row" for="webhook">New webhook URL (write-only)</label>
        <input
          id="webhook"
          class="input mono"
          type="password"
          autocomplete="off"
          placeholder="https://discord.com/api/webhooks/…"
          bind:value={webhook}
        />
        <p class="hint">The URL is never shown again, never logged, and never sent to the webview.</p>
      </fieldset>

      <div class="actions">
        <button type="button" class="btn-secondary" onclick={save} disabled={saving}>
          {saving ? "Saving…" : "Save"}
        </button>
        <button type="button" class="btn-primary" onclick={test} disabled={sending}>
          {sending ? "Sending…" : "Send test message"}
        </button>
        <span role="status" class="result">{result}</span>
      </div>
    </form>
  {/if}
</section>

<style>
  .discord {
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
  .row.col { flex-direction: column; align-items: stretch; }
  .input {
    background: var(--color-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-foreground);
    padding: var(--space-sm) var(--space-md);
    flex: 1;
  }
  .mono { font-family: var(--font-mono); }
  .hint { color: var(--color-muted-foreground); font-size: 12px; }
  .actions { display: flex; align-items: center; gap: var(--space-md); }
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
  .btn-secondary:disabled { opacity: 0.5; cursor: not-allowed; }
  .result { color: var(--color-muted-foreground); font-size: 12px; }
  .error { color: var(--color-destructive); font-size: 12px; }
</style>
