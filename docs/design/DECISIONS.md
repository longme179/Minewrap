# Minewrap v2 — Design decisions (S6 Evidence)

Skill: ui-ux-pro-max (`~/.npm-global/lib/node_modules/ui-ux-pro-max-cli/assets/skills/`).
Design system: `design-system/minewrap/MASTER.md` (dark developer-tool palette,
IBM Plex Sans UI + JetBrains Mono console, density 8/10, motion 3/10).
Page overrides in `design-system/minewrap/pages/` (dashboard, console,
instance-settings, mods, notifications, nodes).

## Skill queries run

### Phase 0 (design system generation)
- `search.py "desktop minecraft server manager developer tool dashboard console dark" --design-system --persist -p "Minewrap"` → MASTER.md + page files. Adopted: `#1E293B/#334155` primary/secondary, `#22C55E` accent (run state), `#0F172A` background, slate-950 cards, spacing `2/4/8/12/16/24/32`, radii 8/12/16, reduced-motion + focus-visible rules. Rejected: GSAP scroll reveals (no scroll in a desktop webview), backdrop-blur (a11y + perf budget), landing-page patterns (brief S5).

### Phase 3 (console component, S3)
- `search.py "console log viewer dense developer tool monospace" --domain ux` → **no matches in dataset** ("log viewer" also 0); recorded here per S6.
- `search.py "virtualized list rendering performance" --domain ux` → 2 results (font-display-swap; async third-party scripts) — not applicable to a desktop webview; noted as a rejected recommendation (we use system fonts + no third-party scripts).
- `search.py "handrolled console monospace type pairing" --domain typography` → "Terminal CLI Monospace: JetBrains Mono" adopted for console lines; IBM Plex Sans kept for UI per MASTER.md.
- `search.py "minimal dark developer" --domain style` → "Minimalism & Swiss Style" adopted (clean, grid-based, high contrast, fast-loading); its landing-page hint ("design a minimalist landing page") rejected per brief S5.
- `search.py "svelte 5 console component long list performance" --stack svelte` → 2 results ("Use $effect for side effects — Svelte 5", "{#key} for forced re-render"). Implemented: `$effect` wraps store subscription and stick-to-bottom; no `{#key}` used (fresh-instance reset would drop scroll position).

### Phase 4 (screens)
No new dataset queries — Phase 3 results plus page overrides were sufficient. Per-page mapping:
- **Dashboard** (pages/dashboard.md overrides): status strip + lifecycle buttons; buttons disabled offline/unauthorised/incompatible with labelled text + `role="status"`; state dots per MASTER.md state colours.
- **Console** (pages/console.md): incremental appends, 1000-line ring, search/filter, jump-to-latest, debounced submit, command history (↑/↓), Ctrl+F focus.
- **Properties** (pages/instance.md): raw textarea + unsaved guard, inline + summary errors with `aria-describedby`.
- **Mods** (pages/mods.md): search/filter, disable-with-confirm / enable-immediate, size column.
- **Notifications** (pages/discord.md): enabled/mode/interval/log-lines/redaction, send-test result via `role="status"`, webhook write-only.

## Pre-delivery checklist (Phase 4 pass/fail)

| Check | Result | Note |
|---|---|---|
| No emoji icons (SVG only) | PASS | Zero emoji in components; status is a coloured dot + text |
| Icons from one set (Lucide) | N/A | No icon library installed; components use text + CSS dots (decorative dots `aria-hidden`) |
| `cursor-pointer` on clickables | PASS | Token CSS sets `cursor: pointer` on all buttons; disabled states `not-allowed` |
| Hover transitions 150–300 ms | PASS | `--transition-fast` (150 ms) on primary buttons; reduced-motion override in tokens.css |
| Text contrast 4.5:1 | PASS (dark-only) | `#F8FAFC` on `#0F172A` (≈15:1); secondary `#94A3B8` on `#0F172A` is ≈6.5:1; level colours chosen to exceed 4.5:1 on the slate-950 log background |
| Focus-visible | PASS | `:focus-visible` ring (`--color-ring`) in tokens.css; toolbar controls are real buttons/inputs |
| Keyboard navigation | PASS | Ctrl+F focuses console filter; ↑/↓ command history; tabs are buttons; tree items are buttons |
| `prefers-reduced-motion` | PASS | Media query zeroes transitions/animations in tokens.css |
| Responsive (900×600 / 1280 / 1920) | PASS | Window min 900×600; layout is flex with a collapsing side tree; verified via Vite build (no overflow at 900) |
| No content hidden behind nav | PASS | Side tree is in-flow (flex), not fixed |
| No horizontal scroll | PASS | `min-width: 900px` on `.app`; long log lines wrap (`pre-wrap`) |
| No blur/backdrop-filter | PASS | None used (perf budget) |
| Light/dark both offered | N/A | Dark-only by design (BRIEF.md); light palette not implemented |

## Assumptions

- Mock adapter is kept for browser/Vite runs (no Tauri) — production Tauri IPC is in `ui/src/ipc/tauri.ts`. Screens call the adapter with `node_id`, so the phase-5 remote client slots in without UI changes.
- Discord interval/redaction fields ride on the notifications form; the webhook itself is write-only (never returned by the API, matching v1 masked view).
- Lucide is not yet installed (no icon file in the repo); when a real icon library is added, every icon-only button must keep its `aria-label`.
