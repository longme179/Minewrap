# Release notes

## v1.10.0 — P0 daemon-first (BREAKING: closing the GUI no longer stops servers)

**The daemon is now the always-on half; the GUI is a pure client.**

- **Breaking change:** closing the Minewrap window only disconnects —
  servers keep running, SLP status keeps updating and Discord keeps
  posting. The old "close GUI = stop everything" behaviour is gone.
  **Migration:** to stop everything, stop the daemon —
  `systemctl --user stop minewrapd` (if running under the user unit)
  or `kill -TERM $(pgrep minewrapd)`. The daemon stops every instance
  gracefully (stdin `stop` → SIGTERM → SIGKILL per instance) and exits.
- The GUI auto-spawns `minewrapd` when `/health` refuses (bundled
  sidecar → `PATH`), detached with its own process group, logging to
  `{data_dir}/daemon.log`; it re-reads the token after a daemon
  restart.
- Daemon gains the services that used to live (or be missing) in the
  GUI: a live state keeper (`GET /instances` now reports real states),
  a background SLP poller (players/version/latency/MOTD are live
  again), and a Discord service (state-change pushes with debounce +
  periodic status pushes honouring `interval_seconds` /
  `first_push_delay_seconds` / `edit-in-place`), all sharing one
  `reqwest::Client`.
- New REST routes (API stays `1.0`, additive): `POST /api/v1/instances`,
  `DELETE /api/v1/instances/{id}`, `GET /api/v1/java`,
  `POST /api/v1/scan-workdir`; the `/events` WebSocket now forwards
  batched log lines (100 lines / 40 ms, `seq`-tagged), `restart_loop`
  and `instances_changed` (it used to drop them).
- `autostart = true` is now honoured by the daemon; owner tasks spawn
  lazily (first start), so offline instances cost no runtime.
- Remote-node credentials persist in `~/.config/minewrap/nodes.json`
  (0600) — the "credential store not wired" stub is gone; every IPC
  command routes to its node (local or remote) over the same client.
- Robustness: a failed server spawn no longer kills the instance's
  owner task (same-state `spawn_failed` event + `last_error` in
  snapshots); the SLP client's response reader was fixed (its happy
  path had never run); `instance_restart` waits for the stop to settle
  before starting again.
- systemd unit: `TOKIO_WORKER_THREADS=2`, `MALLOC_ARENA_MAX=2`.
- Repo hygiene: `build.sh` (build-from-source, `--daemon-only` mode)
  at the root; the repo contains only app code, docs and that script.

## v1.9.3
- Fix orphaned app processes after close: prevent per-instance process-manager runtime threads from blocking forever on exit, harden GUI exit/cleanup flow, and stop the event forwarder when the event bus closes.
- Update project and app license to GNU Affero General Public License v3.0 (AGPL-3.0).
- Refresh app and project logo/icon assets.

## v1.9.2
- Fix RAM (Min/Max) settings being ignored on server start:
  - Always include `-Xms`/`-Xmx` from the configured RAM values (including preset "None").
  - Strip stale `-Xms`/`-Xmx` entries from custom JVM args on load/save so they cannot override the selected RAM (the JVM uses the last occurrence).
  - Parse displayed RAM with last-wins semantics and validate RAM values before saving/creating an instance.
  - Persist daemon config patches to TOML and push the updated config to the running process manager so starts use the new settings.

## v1.9.1
- Clean up repository contents to keep project sources, packaging assets, and user-facing documentation.
- Bump app and crate versions to `1.9.1`.

## v1.9.0
- Fix process manager/runtime issues: valid tokio channel handling, apply updated instance config to the running process manager, and show crash-during-start diagnostics (exit code + log tail).
- Stop local servers/processes on GUI exit so closing the app does not leave Minewrap or managed server processes running in the background.
- Fix Discord webhook embed payload and add best-effort state/status notifications from the GUI.
- Fix Console wiring and log sequencing/backfill so server logs and commands work in the UI.
- UI: add a built-in Close app button and improve spacing/margins.
