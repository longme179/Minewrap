# Minewrap v2 — Design brief (ui-ux-pro-max)

Source of truth for rules is `design-system/minewrap/MASTER.md` (skill
generated, dark developer-tool palette, IBM Plex Sans UI + JetBrains Mono
console, density 8/10, motion 3/10). Page overrides in `pages/` refine it
(dashboard, console, instance, mods, discord, node). This brief adapts the
skill's web/landing bias to a desktop app in a WebKitGTK webview — landing
patterns dropped, minimum window **900×600** (verify 900×600, 1280, 1920),
no browser chrome, no backdrop-filter (performance budget). Desktop is dark-
only; the light palette in MASTER.md is not implemented.

## Shell

- `MainWindow` with three regions: **node/instance tree** (compact, dense),
  **instance page** (status strip, action buttons, tabbed console/settings),
  and a status bar. Tauri window 900×600 min, resizable.
- Global **first-run wizard** (auto-opens with no nodes) and change-only UI
  copies keyring paths, never tokens themselves.
- Keyboard: tree navigation, `Ctrl+F` focus in console search, `Esc` closes
  menus/dialogs; all controls reachable and focus-visible.

## Per-instance pages (12-col shell, cards, 8px/12px/16px spacing)

| Page | Purpose | Key UI |
|---|---|---|
| Dashboard | item selection + quick state | status strip (players, latency, uptime, version), Start/Stop/Restart, tabbed Console/Settings |
| Console | log viewer + command input | JetBrains Mono lines, level colours, ring buffer 1000, search/filter, jump-to-latest, input with history |
| Settings | launch/probe/restart | compact forms, inline + summary errors, Aikar preset, probe opts |
| Properties | `server.properties` editor | typed fields + raw view, unsaved guard, inline errors |
| Mods | `mods/` manager | search/filter, `.jar ↔ .jar.disabled` toggles with confirm, size columns |
| Notifications (Discord) | notifier | enabled/mode/interval/log-lines/redaction, send-test result, rate-limit status |
| Nodes | node list | local auto node + remotes, connection state, reconnect/backoff, unauthorised/incompatible banners |

## Data/state UX (ux skill)

- **Live updates**: batched log batches and state changes streamed via Tauri
  events; the console appends without re-rendering the list. Batch interval
  configurable (30–50 ms typical).
- **Empty/loading/error states**: node offline, unauthorised, incompatible,
  crashed (with reason), unresponsive — each gets a labelled icon + text
  (never colour-only) and `role="status"`/`role="alert"`.
- **Forms**: inline errors bound with `aria-describedby`, error summary at
  top of the form with focus moved there on failed submit, links to fields.
- **Toasts**: async outcomes appear as small notifications; destructive
  actions (stop, remove node, disable mod) require confirmation.
- **Debounced search**: filter and mod search use a small debounce; empty
  results show "No results — try X" with a clear action.
- **Reduced motion**: respect `prefers-reduced-motion`; no smooth reveal /
  animations in the v2 app (motion dial 3 → minimal). No blur/filter.
- **Icons**: Lucide (Phosphor in the dataset) — decorative icons
  `aria-hidden`, icon-only buttons get `aria-label`. No emoji icons.
- **Contrast**: UI text and log-level colours all ≥4.5:1 against the
  slate-950 background; secondary text `#94A3B8` is max lightness allowed.
- **sparklines** (RAM/players/latency) are Phase 6 stretch; when added use
  SVG line (distinct dash + labels, not hue-only) with a text fallback.

## Theme tokens (derived, to implement in `ui/src/styles/tokens.css`)

- Palette from MASTER.md: primary `#1E293B`, secondary `#334155`, accent
  `#22C55E` (green — run state), background `#0F172A`, foreground `#F8FAFC`,
  card `#1B2336`, muted `#272F42`, muted-fg `#94A3B8`, border `#475569`,
  destructive `#EF4444`, ring `rgba(226,232,240,.4)`.
- Type: IBM Plex Sans UI, **JetBrains Mono console**; scale from the master
  spacing tokens (`xs 2, sm 4, md 8, lg 12, xl 16, 2xl 24, 3xl 32`).
- Radius 8/12/16; shadows sm/md/lg/xl; transitions 150–300 ms, subtle only.
- State colours: online green `#22C55E`, starting/stopping amber `#F59E0B`,
  crashed/unresponsive red `#EF4444`, offline grey `#64748B`, matching
  status dots. Log levels: TRACE/DEBUG muted grey, INFO slate-100, WARN amber,
  ERROR red, FATAl bright red, plus system/cyan for daemon notices.
- All components use tokens, never hard-coded values.

## Flows not covered by the skill (design decisions)

- Terminal input history (↑/↓) is kept from v1.0.0; aria: input labelled,
  results not announced as live (the on-screen text is the status).
- "Jump to latest" button appears on scroll-up (pauses auto-scroll).
- Mod disable requires confirm; enable is immediate.
- Properties save guarded when unsaved changes exist.
