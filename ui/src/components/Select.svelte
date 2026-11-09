<!-- Themed, accessible dropdown (replaces native <select>).
  Keyboard: buttons + aria-haspopup/expanded; options as
  role="listbox"/role="option" with aria-selected; Esc closes. -->
<script lang="ts">
  interface Option {
    value: string;
    label: string;
  }

  interface Props {
    value?: string;
    options: Option[];
    placeholder?: string;
    name?: string;
    disabled?: boolean;
    onSelect?: (value: string) => void;
  }

  let {
    value = $bindable(""),
    options = [],
    placeholder = "Select…",
    name = "",
    disabled = false,
    onSelect = () => {},
  }: Props = $props();

  let open = $state(false);
  const label = $derived(options.find((o) => o.value === value)?.label ?? placeholder);

  function choose(v: string) {
    value = v;
    onSelect(v);
    open = false;
  }

  function toggle() {
    if (!disabled) open = !open;
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") open = false;
  }
</script>

<div class="select" class:open data-name={name} data-open={open}>
  <button
    type="button"
    class="trigger"
    aria-haspopup="listbox"
    aria-expanded={open}
    {disabled}
    onclick={toggle}
    onkeydown={onKeydown}
  >
    <span>{label}</span>
    <span class="caret" aria-hidden="true">▾</span>
  </button>
  {#if open}
    <ul class="menu" role="listbox" aria-label={name || "Select"}>
      {#each options as o (o.value)}
        <li>
          <button
            type="button"
            class="item"
            role="option"
            aria-selected={o.value === value}
            onclick={() => choose(o.value)}
          >
            {o.label}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .select { position: relative; display: inline-block; min-width: 180px; }
  .trigger {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-md);
    width: 100%;
    background: var(--color-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-foreground);
    padding: var(--space-sm) var(--space-md);
    cursor: pointer;
  }
  .trigger:disabled { opacity: 0.5; cursor: not-allowed; }
  .trigger:focus-visible { outline: 2px solid var(--color-ring); outline-offset: 2px; }
  .caret { font-size: 10px; opacity: 0.8; }
  .menu {
    position: absolute;
    top: calc(100% + 2px);
    left: 0;
    right: 0;
    list-style: none;
    margin: 0;
    padding: var(--space-xs);
    background: var(--color-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    box-shadow: var(--shadow-md);
    z-index: 10;
    max-height: 240px;
    overflow-y: auto;
  }
  .item {
    display: block;
    width: 100%;
    text-align: left;
    background: transparent;
    border: none;
    color: var(--color-foreground);
    padding: var(--space-sm) var(--space-md);
    border-radius: var(--radius-sm);
    cursor: pointer;
  }
  .item:hover { background: var(--color-muted); }
  .item:focus-visible { outline: 2px solid var(--color-ring); outline-offset: 1px; }
</style>
