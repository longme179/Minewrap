<!-- Console component: batched log viewer with a frontend ring buffer.
  Auto-scroll sticks to the bottom and pauses when the user scrolls up;
  a "jump to latest" control resumes. JetBrains Mono lines, level colours,
  search/filter box. Appends are incremental — the whole list is not
  re-rendered per line. -->
<script lang="ts">
  import { createConsoleStore } from "../stores/console.js";
  import { levelOf, levelClass } from "../stores/console.js";

  interface Props {
    /** External store (optional, testable). */
    store?: ReturnType<typeof createConsoleStore>;
    /** Label for the command input. */
    inputLabel?: string;
    /** Callback for command submission (mock adapter in browser). */
    onCommand?: (command: string) => void;
    /** Placeholder text for the search box. */
    searchPlaceholder?: string;
  }

  let {
    store = createConsoleStore(),
    inputLabel = "Send command",
    onCommand = () => {},
    searchPlaceholder = "Filter logs… (Ctrl+F)",
  }: Props = $props();

  interface LogSnap {
    seq: number;
    ts: number;
    line: string;
  }
  interface StoreSnap {
    lines: LogSnap[];
    firstSeq: number;
    lastSeq: number;
    stick: boolean;
    total: number;
  }

  const initialSnap: StoreSnap = { lines: [], firstSeq: 0, lastSeq: 0, stick: true, total: 0 };
  let snap: StoreSnap = $state(initialSnap);

  $effect(() => {
    const un = store.subscribe((v) => {
      snap = {
        lines: v.lines.map((l): LogSnap => ({ seq: l.seq, ts: l.ts, line: l.line })),
        firstSeq: v.firstSeq,
        lastSeq: v.lastSeq,
        stick: v.stick,
        total: v.total,
      };
    });
    return () => un();
  });

  let scrollEl: HTMLElement | null = $state(null);
  let filter = $state("");
  let command = $state("");
  let history = $state<string[]>([]);
  let historyIndex = $state(-1);
  let jumpVisible = $state(false);

  // Keep the DOM list capped: render only the ring; the store caps at 1000.
  const visible = $derived(
    filter
      ? snap.lines.filter((l: LogSnap) => l.line.toLowerCase().includes(filter.toLowerCase()))
      : snap.lines
  );

  function onScroll() {
    if (!scrollEl) return;
    const nearBottom = scrollEl.scrollHeight - scrollEl.scrollTop - scrollEl.clientHeight < 24;
    store.setStick(nearBottom);
    jumpVisible = !nearBottom;
  }

  function jumpToLatest() {
    if (scrollEl) {
      scrollEl.scrollTop = scrollEl.scrollHeight;
    }
    store.setStick(true);
    jumpVisible = false;
  }

  $effect(() => {
    // Stick to bottom when new lines arrive and stick is on.
    if (snap.stick && scrollEl) {
      scrollEl.scrollTop = scrollEl.scrollHeight;
    }
  });

  function submitCommand(e: Event) {
    e.preventDefault();
    const cmd = command.trim();
    if (!cmd) return;
    onCommand(cmd);
    history.push(cmd);
    historyIndex = -1;
    command = "";
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "ArrowUp") {
      e.preventDefault();
      if (history.length === 0) return;
      historyIndex = Math.max(0, historyIndex === -1 ? history.length - 1 : historyIndex - 1);
      command = history[historyIndex] ?? "";
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      if (historyIndex === -1) return;
      historyIndex = Math.min(history.length - 1, historyIndex + 1);
      command = history[historyIndex] ?? "";
      if (historyIndex === history.length - 1) historyIndex = -1;
    } else if (e.key === "f" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      document.querySelector<HTMLInputElement>("[data-console-filter]")?.focus();
    }
  }

  function fmtTs(ts: number): string {
    try {
      const d = new Date(ts);
      return d.toISOString().slice(11, 19);
    } catch {
      return "";
    }
  }
</script>

<section aria-label="Console" class="console">
  <div class="toolbar" role="toolbar" aria-label="Console controls">
    <label>
      <span class="sr-only">Search logs</span>
      <input
        data-console-filter
        type="search"
        bind:value={filter}
        placeholder={searchPlaceholder}
        aria-label="Filter logs"
        class="input"
      />
    </label>
    {#if jumpVisible}
      <button type="button" class="btn-secondary" onclick={jumpToLatest}>
        Jump to latest
      </button>
    {/if}
    <span class="count" role="status" aria-live="off">
      {visible.length} / {snap.total} lines
    </span>
  </div>

  <div bind:this={scrollEl} onscroll={onScroll} class="log" role="log" aria-label="Server logs">
    {#if visible.length === 0}
      <p class="empty">{filter ? "No results — try clearing the filter." : "No log lines yet."}</p>
    {:else}
      {#each visible as line (line.seq)}
        <div class="row" data-seq={line.seq}>
          <span class="ts">{fmtTs(line.ts)}</span>
          <span class="text {levelClass(levelOf(line.line))}">{line.line}</span>
        </div>
      {/each}
    {/if}
  </div>

  <form class="cmd" onsubmit={submitCommand}>
    <label for="console-input" class="sr-only">{inputLabel}</label>
    <input
      id="console-input"
      bind:value={command}
      onkeydown={onKeydown}
      placeholder="Type a command (↑/↓ history)…"
      class="input mono"
      autocomplete="off"
      spellcheck="false"
    />
    <button type="submit" class="btn-primary">Send</button>
  </form>
</section>

<style>
  .console {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--color-background);
    color: var(--color-foreground);
    font-family: var(--font-sans);
  }
  .toolbar {
    display: flex;
    gap: var(--space-md);
    align-items: center;
    padding: var(--space-md);
    border-bottom: 1px solid var(--color-border);
  }
  .input {
    background: var(--color-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-foreground);
    padding: var(--space-sm) var(--space-md);
    font-size: 14px;
  }
  .input:focus {
    border-color: var(--color-accent);
    outline: none;
  }
  .mono {
    font-family: var(--font-mono);
  }
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
  .btn-primary:hover { opacity: 0.9; }
  .btn-secondary {
    background: transparent;
    color: var(--color-foreground);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: var(--space-sm) var(--space-md);
    font-weight: 600;
    cursor: pointer;
  }
  .count { margin-left: auto; color: var(--color-muted-foreground); font-size: 12px; }
  .log {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-sm) var(--space-md);
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 1.4;
  }
  .row { display: flex; gap: var(--space-md); padding: 1px 0; }
  .ts { color: var(--color-muted-foreground); flex-shrink: 0; }
  .text { white-space: pre-wrap; word-break: break-word; }
  .level-trace, .level-debug { color: var(--color-muted-foreground); }
  .level-info { color: #f8fafc; }
  .level-warn { color: #f59e0b; }
  .level-error { color: #ef4444; }
  .level-fatal { color: #f87171; font-weight: 700; }
  .cmd {
    display: flex;
    gap: var(--space-md);
    padding: var(--space-md);
    border-top: 1px solid var(--color-border);
  }
  .cmd .input { flex: 1; }
  .empty { color: var(--color-muted-foreground); padding: var(--space-lg); }
  .sr-only {
    position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px;
    overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; border: 0;
  }
</style>
