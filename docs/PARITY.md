# Minewrap v2 — Parity audit (Python v1.0.0 → Rust/Svelte)

Source of audit: `packages/mcpanel-{common,daemon,gui}` (34 Python files,
5,427 lines) and `tests/` (21 files, ~146 test functions, reported 151 tests
with parametrized cases). Version facts: package `version = "1.0.0"`,
`API_VERSION = "0.1"` (`packages/mcpanel-common/src/mcpanel_common/version.py`).

Every item lists its v2 target. "Port" = same behaviour re-implemented in
`minewrap-core`; "New" = no v1 counterpart; "Drop" = intentionally not carried.

## 1. Configuration schema and store

| Feature | v1.0.0 location | v2 target |
|---|---|---|
| `DaemonSettings` (host, port, data_dir, token, poll interval, unresponsive threshold, defaults) | `mcpanel_common/config.py:58-75` | `minewrap-core/src/config.rs` (same fields/names) |
| `InstanceDefaults` (startup/stop/kill timeouts, done_regex, log caps/backups) | `config.py:34-43` | `config.rs` (same fields, `#[serde(deny_unknown_fields)]`) |
| `ProbeConfig` (host, port, query_enabled, query_port, timeout) | `config.py:78-94` | `config.rs` |
| `RestartConfig` (never/on-crash, retries, backoff, reset_after) | `config.py:96-104` | `config.rs` |
| `RconConfig` (enabled, host, port, password_env) — reserved, unimplemented | `config.py:106-112` | `config.rs` (carried, still unimplemented) |
| `DiscordConfig` (enabled, webhook_url, mode, interval, first_push, log_lines, max_log_chars, state_change_posts, debounce, redact_regexes) | `config.py:114-132` | `config.rs`; webhook moves to keyring/0600 store |
| `InstanceConfig` (id, name, work_dir, java_path, jvm_args, launch_args, autostart, probe, restart, rcon, discord) | `config.py:134-160` | `config.rs` |
| Schema validation: strict `extra="forbid"`, ranges, id regex, unique ids, redact-regex compilation | `config.py` + `test_config_validation` (12 tests) | `config.rs` validation + `#[cfg(test)]` unit tests |
| Config loader: relative paths resolved against config file dir | `config_store.py` | `config.rs::load` |
| Atomic write-back (write tmp, chmod 0600, replace) | `config_store.py:60-68` | `config.rs::save` (same pattern) |
| Token: explicit or generated+printed once into `data/token` (0600) | `config_store.py:82-102` | `config.rs` / secrets store (keyring first, 0600 fallback) |
| `InstanceConfigView` (secrets masked, `webhook_set: bool`) | `mcpanel_common/api.py:158-185` | `minewrap-core/src/api.rs` DTOs |
| `ConfigUpdateRequest` (write-only `webhook_url`, `None` keeps) | `api.py:123-155` | `api.rs` |

## 2. Process manager (state machine + lifecycle)

| Feature | v1.0.0 location | v2 target |
|---|---|---|
| ProcessState `offline/starting/online/stopping/crashed` | `process_manager.py:33-38` | `minewrap-core::process::State` |
| Transition set (OFFLINE→STARTING; STARTING→ONLINE/STOPPING/CRASHED; ONLINE→STOPPING/CRASHED; STOPPING→OFFLINE; CRASHED→STARTING/OFFLINE) | `process_manager.py:41-47` | `process.rs` table + tests |
| Single `transition()`, logged + event on topic | `process_manager.py:245-270` | `process.rs` + `events.rs` |
| Spawn: `create_subprocess_exec`, own session/process group, stdin/stdout piped, stderr merged | `process_manager.py:175-208` | `process.rs` via `tokio::process::Command`, `setsid` |
| `starting→online` on done-regex (configurable default), startup timeout → crashed | `process_manager.py:231-250` | `process.rs` |
| Stop: `stop` on stdin, wait `stop_timeout`, SIGTERM, wait `kill_grace_seconds`, SIGKILL | `process_manager.py:139-173` | `process.rs` |
| Crash vs requested stop (`requested_stop` flag) | `process_manager.py:276-306` | `process.rs` |
| Send-command: stdin write, reject unless ONLINE, single-line enforcement | `process_manager.py:182-194` | `process.rs` (same guards) |
| Restart policy: exponential backoff (`backoff_base * 2^(n-1)` capped), `max_retries`, then restart-loop event (stable CRASHED) | `process_manager.py:134-136, 302-327` | `process.rs` + event |
| Crash counter resets after `reset_after_online_seconds` online | `process_manager.py:112-120` | `process.rs` |
| Manual `stop` on CRASHED acknowledges → OFFLINE; manual `start` clears counters | `process_manager.py:117, 122-132` | `process.rs` |
| Known limit: child dies with daemon (no detached mode) | `process_manager.py:7` | still documented (ADR-2) |
| UTF-8 garbage in output handled (`errors="replace"`) | `log_buffer.py`/`process_manager.py` | `logs.rs` (`String::from_utf8_lossy`) |

## 3. Log pipeline

| Feature | v1.0.0 location | v2 target |
|---|---|---|
| `LogBuffer` ring: appended `LogLine(seq, ts, line)`, monotonic seq, `tail`/`since`/`first_seq`/`last_seq` | `log_buffer.py` (4 tests) | `logs.rs::RingBuffer` (unit tests, port tests) |
| Rotating file tee (size-based, backups) | `log_file.py` (3 tests) | `logs.rs::RotatingFile` |
| `since_seq` backfill + `caught_up` on connect | `ws.py` (REST `logs` + WS hello) | `logs.rs` + Tauri/axum (api surface) |
| Cap + dedupe of duplicate lines after backfill | gui `console_widget.py` | ui console store (port `ConsoleModelTests`) |
| `resync` on slow consumers (drop-oldest bounded queues) | `events.py` (5 tests) | `events.rs` (bounded mpsc) |
| PySide6 GUI console: 5000-line model, plain-text widget, pause-on-scroll-up | `console_widget.py`, `viewmodels.py:ConsoleModel` | **New**: batched events + frontend ring (1000 default), search/filter, level colours → *not* 1:1 |
| Status probe (mcstatus `async_status`, latency, MOTD, version, players; optional `async_query` for names) | `poller.py` (6 tests) | `poller.rs` (hand-rolled SLP, see §6) |
| `McstatusProber` maps status/query responses; query failure keeps status | `poller.py` (3 mapping tests) | `poller.rs` |
| `InstanceState` merges process + probe → combined `online/unresponsive/…` (threshold N) | `snapshot.py` (6 tests) | `snapshot.rs` (same logic, port tests) |
| `ProcessStatus`/`InstanceSnapshot`/`InstanceSummary` shapes | `snapshot.py` + `api.py` | `api.rs` DTOs (ported field-for-field) |

## 4. Event bus + per-instance wiring

| Feature | v1.0.0 | v2 target |
|---|---|---|
| `EventBus` topics + wildcard, bounded queues, drop-oldest | `events.py` (5 tests) | `events.rs` (mpsc-based) |
| `InstanceManager` builds per-instance buffer/file/state/process/poller/notifier; autostart; background task error guard | `instances.py` | `core::run_instance` / `instances.rs` in axum crate |
| `instance.request_start/stop/restart` pre-checks → HTTP 409 on wrong state | `instances.py:120-136`, `app.py` | command layer returns typed errors |

## 5. Discord notifier

| Feature | v1.0.0 | v2 target |
|---|---|---|
| `redact_line`: strip ANSI, login-form IP `[/1.2.3.4:5678]`, bare IPv4/IPv6, user regexes, truncate long lines (300 chars) | `redact.py` (4 tests incl. keep `12:34:56` timestamps) | `discord.rs::redact` (regex crate subset, port tests) |
| `build_status_embed`: title `name — state`, colour per state, version/players(+names)/latency/uptime/last-updated fields, fenced log block | `discord.py` (2 tests) | `discord.rs` |
| Log block trimming: oldest-first drop to `max_log_chars`, code-fence escape | `discord.py` (2 tests) | `discord.rs` |
| `push_status_once`: edit-in-place create `?wait=true` → persist `message_id` → PATCH; 404 → recreate; `new-message` mode | `discord.py` (3 tests) | `discord.rs` |
| 429 `retry_after` (min vs cap 60 s), 5xx exponential backoff, connection-error → one masked log line | `discord.py` (3 tests) | `discord.rs` |
| `mask_webhook_url` in logs | `discord.py` (1 test) | `discord.rs` |
| `humanize_uptime` | `discord.py` | `discord.rs` |
| Periodic loop (first push after `first_push_delay`, then `interval`) + state-change debounce (extra post, coalesced) | `discord.py` | `discord.rs` |
| `send_test` → `DiscordTestResponse` | `discord.py` (2 tests) | `discord.rs` (IPC `discord_test`) |

## 6. HTTP/WebSocket API parity

| Endpoint / behaviour | v1.0.0 (`app.py`, `ws.py`) | v2 target |
|---|---|---|
| `GET /health` (public) → `{status, api_version, daemon_version}` | `app.py` | `crates/minewrapd/src/api.rs`, api version **1.0** |
| Bearer 401, constant-time compare, WS handshake denial | `auth.py` (2 tests), `app.py` | `minewrapd` middleware (parity), plus ref-count at REST level |
| `GET /api/v1/instances` list summaries | `app.py` | `minewrapd` |
| `GET /api/v1/instances/{id}` snapshot | `app.py` | `minewrapd` |
| `POST …/{start,stop,restart,command}` → `ActionResponse` / `CommandResponse`, 409 on wrong state | `app.py` | `minewrapd` |
| `GET …/logs?limit&since_seq` → `LogsResponse` | `app.py`, `instances.py` | `minewrapd` |
| `GET/PUT …/config` (masked get, write-only webhook, persist TOML 0600, poller/notifier rebuild) | `app.py`, `instances.py` | `minewrapd` |
| `POST …/discord/test` | `app.py`, `instances.py` | `minewrapd` |
| Console WS: `hello`/`command` in; `log`/`caught_up`/`resync`/`snapshot` (incl. snapshot on every connect)/`command_result`/`error` out; 401 handshake denial | `ws.py`, `test_api.py` (16 tests) | `minewrapd` (same shapes) |
| Events WS: `hello_ok`, `state_change` | `ws.py`, `test_api.py` | `minewrapd` |
| OpenAPI schema generated & committed (REST paths) | `openapi.py`, `docs/openapi.json` (5 contract tests) | `minewrapd` generates `/openapi.json` (or utoipa) |
| WS message model round-trips (discriminated unions) | `api_models` (8 tests) | `api.rs` serde tagged enums + tests |

## 7. GUI parity (PySide6 → drop or reimplement)

| v1.0.0 feature | Location | v2 target |
|---|---|---|
| `ApiClient` (httpx REST + websockets, version check, http→ws scheme) | `apiclient.py` (8 integration tests) | **Drop** — replaced by Tauri commands; `minewrap-remote` is the remote client anyway |
| `NodeStore` (nodes.json 0600, `normalize_url`) | `node_store.py` (2+1 tests) | Tauri-side node registry (`models/node.rs`) with same URL rules |
| Token storage (keyring → 0600 file fallback) | `node_store.py:TokenStore` | `minewrap-core` secrets module (keyring crate or file) |
| `NodeViewModel`/`InstanceViewModel` connection + summary/snapshot merge | `viewmodels.py` (3 tests) | Svelte stores (desktop only) |
| `ConsoleModel` (dedupe by seq, cap, history with prev/next) | `viewmodels.py` (2 tests) | ui `stores/console` (cap default **1000** — new default) |
| Worker threadpool for off-thread REST calls | `workers.py` | Rust async commands (no-op equivalent) |
| `session.py` NodeSession (WS reconnect/backoff, 401/incompatible/offline states) + `ConsoleStream` (backfill, resync, command round trip) | `session.py` | Tauri event loop + `minewrap-remote`; semantics ported |
| Widgets: main window (tree + pages), instance page (status, Start/Stop/Restart, tabs), console (pause-on-scroll-up, history), settings (launch/probe/restart/Discord), local setup wizard (jar required, daemon spawn detached) | `widgets/` (7 smoke tests) | Svelte UI (design system); wizard required behaviour ported (jar validation, daemon binary discovery incl. foreign-APPIMAGE guard) |
| Local setup wizard: schema-valid config write 0600, binary discovery (AppDir/dist/PATH/AppImage re-exec with APPDIR containment), spawn daemon, wait `/health`, read token, verify | `daemon_launcher.py` (14 tests) | **Replaced**: wizard writes config and (optionally) spawns `minewrapd`; local node is embedded by default (ADR-2) |

## 8. Packaging / docs

| Feature | v1.0.0 | v2 target |
|---|---|---|
| PyInstaller onefile daemon + one-folder GUI; AppRun dispatches `--daemon` (bundled daemon in `usr/bin`) | `packaging/pyinstaller/*.spec`, `packaging/appimage/AppRun` | Tauri bundler (deb/AppImage) + separate `minewrapd` binary; `packaging/` slimmed |
| `config.example.toml` | repo root | `examples/minewrap.toml` + same defaults |
| `packaging/mcpanel-daemon.service` (systemd user unit) | `packaging/` | `packaging/minewrapd.service` |
| `docs/remote-access.md`, `docs/discord.md`, `ARCHITECTURE.md` | `docs/` | carried over, updated for v2 paths |

## 9. Tests that carry over (by file, v1 → v2)

| v1 file | count | v2 home |
|---|---|---|
| `test_process_manager.py` (13 async) | 13 | `minewrap-core` `process` tests (fake-server fixture) |
| `test_poller.py` (6 async) | 6 | `poller.rs` tests (mock SLP) |
| `test_snapshot.py` | 6 | `snapshot.rs` tests |
| `test_log_buffer.py` | 4 | `logs.rs` ring tests |
| `test_log_file.py` | 3 | `logs.rs` rotation tests |
| `test_events.py` | 5 | `events.rs` tests |
| `test_discord.py` (10 async + 10 pure) | 20 | `discord.rs` + mock-Discord-server tests |
| `test_api.py` (16), `test_api_models.py` (8), `test_openapi_contract.py` (5), `test_auth.py` (2) | 31 | `minewrapd` integration tests (axum) |
| `test_config_validation.py` (12), `test_config_store.py` (7) | 19 | `config.rs` tests + migration tests |
| `test_gui_models.py` (8), `test_gui_smoke.py` (7), `test_gui_apiclient.py` (8), `test_daemon_launcher.py` (14) | 37 | Svelte stores/unit + mock IPC adapter tests |
| `test_shutdown.py` (2 async) | 2 | graceful-shutdown integration test |

~146 test functions (151 reported tests); modular pytest count reconciles
with the 151 that CI reported. v1 used `pytest`/`pytest-asyncio` (31 async
tests across 4 files). v2 keeps the same coverage via `cargo test`
(core + axum) and `pnpm` unit tests (stores/components).

## 10. New in v2 (no v1 counterpart)

- **Log batching + frontend ring**: batch interval ~30–50 ms (or N lines),
  frontend cap 1000 default; search/filter; level colours; jump-to-latest.
- **`server.properties` round-trip parser** preserving comments/order/unknown
  keys, typed editor with raw view + unsaved guard.
- **Mods manager**: list/search in `mods/`, `.jar ↔ .jar.disabled` toggle,
  size/name. NeoForge jar metadata is stretch.
- **Aikar's flags preset** for JVM args in the launch editor.
- **System tray** + close-to-tray (best-effort on Wayland, honest limits).
- **First-run AIO**: single AppImage = GUI + daemon; wizard auto-opens.
- **Mock IPC adapter** so the Svelte UI runs in a browser (Vite dev) with live
  fake data for screenshots.

## 11. Not in v1 (parity check closed)

- No backups feature exists in v1.0.0 → not parity.
- No `server.properties` editor, no mods manager in v1 → new (listed above).
- No RCON TPS implemented in v1 → carried as config only.
