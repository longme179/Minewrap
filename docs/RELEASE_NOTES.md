# Minewrap v2.0.0 — Release notes

First Rust/Svelte release. Replaces the Python/PySide6 v1.0.0 (`legacy/`,
still runnable under tag `v1.0.0`).

## Added
- Tauri 2 + Svelte 5 + TypeScript desktop app: node/instance tree, dashboard
  (status strip + lifecycle), console (batched logs, 1000-line ring,
  search/filter, level colours, jump-to-latest, Ctrl+F, history), properties
  editor (raw round-trip + unsaved guard), mods manager (toggle
  `.jar ↔ .jar.disabled`), Discord notifications (write-only webhook), nodes
  (local auto node + remotes with connection state/reconnect).
- `minewrap-core` — Tauri-free core shared by both binaries: config,
  process/state machine, log pipeline (batching + ring + rotation), SLP
  status probe (hand-rolled), Discord notifier (edit-in-place, 429/5xx
  retries, IP/ANSI redaction), properties/mods with path confinement.
- `minewrapd` — headless axum daemon (REST + WS) binding 127.0.0.1 by
  default, bearer-token auth (401 at REST + WS handshake denial), token
  generated once into `0600` (`data/token`), `openapi.json`.
- `minewrap-remote` — reqwest client for remotes (unauthorised +
  incompatible-version states).
- Aikar's Flags preset (`minewrap_core::aikars_flags()`).
- systemd user unit `packaging/minewrapd.service`, example config
  `examples/minewrap.toml`, RSS measurement `scripts/measure-rss.sh`,
  `docs/remote-access.md` (SSH port-forward recommended).

## Parity with v1.0.0
`docs/PARITY.md` audits every v1 feature/API/test; all ported items are
`minewrap-core` / `minewrapd` / Svelte screens above.

## Known limitations
- System tray is best-effort on Wayland (StatusNotifier may not show on
  GNOME without an extension); the systemd unit is the reliable
  always-on path. Honestly reported, not hidden.
- The 30 MB RSS budget is measured with `scripts/measure-rss.sh` and
  reported honestly (main vs WebKit helpers vs total).

## Test results
`cargo test --workspace` (41 core + daemon auth), `pnpm test` (3),
`svelte-check` clean, `cargo check` with warnings denied.
