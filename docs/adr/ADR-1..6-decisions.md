# Minewrap v2 — architecture decision records

## ADR-1: hand-rolled the Server List Ping (no mcstatus- equivalent crate)

We ported v1's mcstatus `status()`/`query()` as a small in-core client
(~200 lines: handshake packet → status request → JSON response parse).
Justification: no well-maintained general Rust crate for SLP is available;
the protocol is tiny and size/timeout limits are easy to enforce; avoids a
network of transitive dependencies in a binary with a strict RSS budget.
Query is gated behind `query_enabled` exactly like v1.

## ADR-2: close-to-tray is best-effort; systemd `minewrapd` is the guarantee

Tauri's tray uses StatusNotifier (KSNI). On GNOME without a shell extension
the icon is invisible, and Minewrap does not ship one. Closing-to-tray is
therefore not a supported persistence mechanism. `minewrapd` + systemd user
unit is the supported always-on path. Documented in `docs/remote-access.md`.

## ADR-3: Tauri 2 + Svelte 5 + Tailwind CSS 4, pnpm

Stack majors pinned at implementation start. Tauri 2 is current and matches
the `core`, `api`, `cli` and `bundler` v2 APIs. Tailwind 4 for tokens-via-CSS
(`@theme`), chosen over CSS-in-JS for the performance budget. Svelte 5
runes for stores (nodes/instances/console). Icon set: Lucide-compatible
Phosphor exports (dataset choice, single SVG set — no emoji icons).

## ADR-4: API 1.0 (explicit breaking change from v1's 0.1)

v1.0.0's `API_VERSION = "0.1"`; v2's `minewrapd` is **`1.0`**. The shapes are
near-identical (`InstanceSnapshot`, `LogLine`, `LogsResponse`, config view,
WS shapes ported from `mcpanel_common/api.py`), and the GUI maps both states
locally and remotely through the same DTOs. The bump is explicit because the
runtime changes (batching, health fields, faster error codes) from v1.

## ADR-5: secrets in the OS keyring (Secret Service) with 0600 fallback

Node tokens and webhook URLs go to the OS keyring when available, else a
`0600` file. Rationale: v1 required the token in a `0600` file or printed
(transferable to the GUI); keeping secrets out of the webview/config-file
text is a strict improvement. RCON remains never-exposed.

## ADR-6: tray limits reported honestly

We do not game the 30 MB RSS number: reporting splits main Tauri RSS,
WebKit helper RSS and total; the webview's console ring (1000 lines) and
Tauri's allocator are the expected contributors. If the budget is missed,
we report and exclude only documented features (blur effects, sparkline
extras).
## ADR-2: close-to-tray is best-effort; systemd `minewrapd` is the guarantee

Tauri's tray uses StatusNotifier (KSNI). On GNOME without a shell extension
the icon is invisible, and Minewrap does not ship one. Closing-to-tray is
therefore not a supported persistence mechanism. `minewrapd` + systemd user
unit is the supported always-on path. Documented in `docs/remote-access.md`.

## ADR-3: Tauri 2 + Svelte 5 + Tailwind CSS 4, pnpm

Stack majors pinned at implementation start. Tauri 2 is current and matches
the `core`, `api`, `cli` and `bundler` v2 APIs. Tailwind 4 for tokens-via-CSS
(`@theme`), chosen over CSS-in-JS for the performance budget. Svelte 5
runes for stores (nodes/instances/console). Icon set: Phosphor (dataset
choice, single SVG set — no emoji icons).

## ADR-4: API 1.0 (explicit breaking change from v1's 0.1)

v1.0.0's `API_VERSION = "0.1"`; v2's `minewrapd` is **`1.0`**. The shapes are
near-identical (`InstanceSnapshot`, `LogLine`, `LogsResponse`, config view,
WS shapes ported from `mcpanel_common/api.py`), and the GUI maps both states
locally and remotely through the same DTOs. The bump is explicit because the
runtime changes (batching, health fields, faster error codes) from v1.

## ADR-5: secrets in the OS keyring (Secret Service) with 0600 fallback

Node tokens and webhook URLs go to the OS keyring when available, else a
`0600` file. Rationale: v1 required the token in a `0600` file or printed
(transferable to the GUI); keeping secrets out of the webview/config-file
text is a strict improvement. RCON remains never-exposed.

## ADR-6: tray limits reported honestly

We do not game the 30 MB RSS number: reporting splits main Tauri RSS,
WebKit helper RSS and total; the webview's console ring (1000 lines) and
Tauri's allocator are the expected contributors. If the budget is missed,
we report and exclude only documented features (blur effects, sparkline
extras).
