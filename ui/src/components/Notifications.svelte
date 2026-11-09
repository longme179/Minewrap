<!-- Notifications (Discord): enabled / mode / interval / log-lines /
  redaction, send-test result, rate-limit status. Webhook is write-only —
  only `webhook_set` is shown, never the URL. -->
<script lang="ts">
  import { createAdapter } from "../ipc/factory.js";

  interface Props {
    nodeId?: string;
    instanceId?: string;
  }

  let { nodeId = "local", instanceId = "survival" }: Props = $props();

  const ipc = createAdapter();

  let enabled = $state(true);
  let mode = $state("edit-in-place");
  let interval = $state(600);
  let logLines = $state(15);
  let webhookSet = $state(true);
  let webhook = $state("");
  let result = $state("");
  let sending = $state(false);

  async function test() {
    sending = true;
    try {
      const res = await ipc.discordTest(nodeId, instanceId);
      result = res.ok ? "Test message sent." : `Failed: ${res.error ?? "unknown"}`;
    } catch (e) {
      result = String(e);
    } finally {
      sending = false;
    }
  }
</script>

<section aria-label="Discord notifications" class="discord">
  <h2>Discord</h2>

  <form class="form" onsubmit={(e) => e.preventDefault()}>
    <fieldset>
      <legend>Notifier</legend>
      <label class="row">
        <input type="checkbox" bind:checked={enabled} />
        Enabled
      </label>
      <label class="row">
        Mode
        <select bind:value={mode} class="input">
          <option value="edit-in-place">Edit in place (one message, updated)</option>
          <option value="new-message">New message every interval</option>
        </select>
      </label>
      <label class="row">
        Interval (seconds)
        <input class="input" type="number" min="10" bind:value={interval} />
      </label>
      <label class="row">
        Log lines per message
        <input class="input" type="number" min="0" max="50" bind:value={logLines} />
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
      <button type="button" class="btn-primary" onclick={test} disabled={sending}>
        {sending ? "Sending…" : "Send test message"}
      </button>
      <span role="status" class="result">{result}</span>
    </div>
  </form>
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
  .form { display: flex; flex-direction: column; gap: var(--space-lg); }
  fieldset { border: 1px solid var(--color-border); border-radius: var(--radius-sm); padding: var(--space-md); }
  legend { padding: 0 var(--space-sm); color: var(--color-muted-foreground); font-size: 12px; }
  .row { display: flex; align-items: center; gap: var(--space-md); margin-bottom: var(--space-sm); }
  .input {
    background: var(--color-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-foreground);
    padding: var(--space-sm) var(--space-md);
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
  .result { color: var(--color-muted-foreground); font-size: 12px; }
</style>
