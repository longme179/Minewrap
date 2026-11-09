<!-- Dashboard / instance view: status strip, lifecycle actions, tabbed
  Console / Settings / Properties / Mods / Notifications. Buttons are
  disabled in the offline/unauthorised/incompatible states with a
  labelled icon + text (never colour-only) and `role="status"`
  messages. -->
<script lang="ts">
  import Console from "./Console.svelte";
  import Settings from "./Settings.svelte";
  import Properties from "./Properties.svelte";
  import Notifications from "./Notifications.svelte";
  import Mods from "./Mods.svelte";

  export interface DashboardProps {
    snapshot?: {
      id: string;
      name: string;
      combined_state: string;
      players_online?: number | null;
      players_max?: number | null;
      version?: string | null;
      probe_ok: boolean;
      last_error?: string | null;
    } | null;
    nodeState?: string;
    nodeId?: string;
    instanceId?: string;
    tab?: string;
    consoleStore?: any;
    onCommand?: (cmd: string) => void;
    onStart?: () => void;
    onStop?: () => void;
    onRestart?: () => void;
    onTabChange?: (tab: string) => void;
  }

  let {
    snapshot = null,
    nodeState = "connected",
    nodeId = "local",
    instanceId = "survival",
    tab = $bindable("console"),
    consoleStore = undefined,
    onCommand = () => {},
    onStart = () => {},
    onStop = () => {},
    onRestart = () => {},
    onTabChange = () => {},
  }: DashboardProps = $props();

  const stateLabel = $derived(snapshot?.combined_state ?? "unknown");
  const disabled = $derived(
    nodeState === "offline" || nodeState === "unauthorised" || nodeState === "incompatible"
  );

  function select(t: string) {
    tab = t;
    onTabChange(t);
  }

  const unknown = "—";
  const playersText = $derived(
    snapshot?.players_online != null && snapshot?.players_max != null
      ? `${snapshot.players_online}/${snapshot.players_max} players`
      : unknown
  );
  const versionText = $derived(snapshot?.version ?? unknown);

</script>

<section aria-label="Dashboard" class="dashboard">
  <div class="status-strip" role="status" aria-live="polite">
    <span class="status-dot" data-state={stateLabel} aria-hidden="true"></span>
    <span class="status-text">
      {snapshot
        ? `${snapshot.name} — ${stateLabel} · ${playersText} · ${versionText}`
        : "No instance selected"}
    </span>
    <span class="actions">
      <button type="button" class="btn-primary" onclick={onStart} disabled={disabled || stateLabel === "online"}>
        Start
      </button>
      <button type="button" class="btn-secondary" onclick={onStop} disabled={disabled || stateLabel === "offline"}>
        Stop
      </button>
      <button type="button" class="btn-secondary" onclick={onRestart} disabled={disabled}>
        Restart
      </button>
    </span>
  </div>

  {#if snapshot?.last_error}
    <p role="alert" class="snapshot-error">{snapshot.last_error}</p>
  {/if}

  <nav aria-label="Instance tabs" class="tabs">
    <button type="button" role="tab" aria-selected={tab === "console"} class="tab" class:tab-active={tab === "console"} onclick={() => select("console")}>Console</button>
    <button type="button" role="tab" aria-selected={tab === "settings"} class="tab" class:tab-active={tab === "settings"} onclick={() => select("settings")}>Settings</button>
    <button type="button" role="tab" aria-selected={tab === "properties"} class="tab" class:tab-active={tab === "properties"} onclick={() => select("properties")}>Properties</button>
    <button type="button" role="tab" aria-selected={tab === "mods"} class="tab" class:tab-active={tab === "mods"} onclick={() => select("mods")}>Mods</button>
    <button type="button" role="tab" aria-selected={tab === "notifications"} class="tab" class:tab-active={tab === "notifications"} onclick={() => select("notifications")}>Notifications</button>
  </nav>

  <div class="tab-panels">
    {#if tab === "console"}
      <Console store={consoleStore} {onCommand} />
    {:else if tab === "settings"}
      <Settings {nodeId} {instanceId} />
    {:else if tab === "properties"}
      <Properties {nodeId} {instanceId} />
    {:else if tab === "mods"}
      <Mods {nodeId} {instanceId} />
    {:else if tab === "notifications"}
      <Notifications {nodeId} {instanceId} />
    {/if}
  </div>
</section>

<style>
  .dashboard {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--color-background);
    color: var(--color-foreground);
    font-family: var(--font-sans);
  }
  .status-strip {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    padding: var(--space-md) var(--space-lg);
    border-bottom: 1px solid var(--color-border);
  }
  .status-dot { width: 10px; height: 10px; border-radius: 50%; background: var(--color-muted-foreground); }
  .status-dot[data-state="online"] { background: var(--color-state-online); }
  .status-dot[data-state="starting"],
  .status-dot[data-state="stopping"] { background: var(--color-state-starting); }
  .status-dot[data-state="crashed"],
  .status-dot[data-state="unresponsive"] { background: var(--color-state-crashed); }
  .status-text { flex: 1; font-weight: 600; }
  .actions { display: flex; gap: var(--space-sm); }
  .btn-primary {
    background: var(--color-accent);
    color: var(--color-on-accent);
    border: none;
    border-radius: var(--radius-sm);
    padding: var(--space-sm) var(--space-lg);
    font-weight: 600;
    cursor: pointer;
    transition: opacity var(--transition-fast);
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
  .tabs { display: flex; gap: var(--space-xs); padding: var(--space-sm) var(--space-lg); border-bottom: 1px solid var(--color-border); }
  .tab {
    background: transparent;
    border: none;
    color: var(--color-muted-foreground);
    padding: var(--space-sm) var(--space-md);
    border-radius: var(--radius-sm);
    cursor: pointer;
    font-weight: 600;
  }
  .tab-active { color: var(--color-foreground); background: var(--color-muted); }
  .tab:disabled { opacity: 0.5; cursor: not-allowed; }
  .tab-panels { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .tab-panels > :global(*) { flex: 1; min-height: 0; }
  .snapshot-error {
    margin: 0 var(--space-lg) var(--space-sm);
    padding: var(--space-sm) var(--space-md);
    border: 1px solid var(--color-destructive);
    border-radius: var(--radius-sm);
    color: var(--color-destructive);
    font-size: 12px;
    white-space: pre-wrap;
  }
</style>
