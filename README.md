# mcpanel

A manager for Minecraft Java Edition servers: a headless **node daemon** that runs
on any host, and a desktop **GUI client** that manages one or many nodes, local or
remote.

- Start/stop/restart a server, watch and use its console, see live status — all from
  the GUI.
- Independently of the GUI, the daemon pushes server state plus recent log lines to
  a Discord webhook every 10 minutes (configurable).
- The GUI never touches a Minecraft server directly: it only talks to the daemon
  over the API, so "local" (127.0.0.1) and "remote" (via SSH tunnel) are the same
  thing.

## Status

**Phase 7 — hardening, packaging, examples (v0.1 complete).** This repository
currently contains:

- `ARCHITECTURE.md` — design, state machine, data flow, security model.
- `packages/mcpanel_common` — the pydantic models both sides share: the daemon
  config schema and the full API contract (REST DTOs + WebSocket messages).
- `packages/mcpanel_daemon` — the log ring buffer (with rotating file tee),
  event bus, process manager (spawn/stop/restart, crash/restart policy,
  startup timeout, SIGTERM/SIGKILL escalation), the mcstatus status poller
  (async status/query, probe errors never escape the loop), merged instance
  state, and the FastAPI daemon: bearer auth, REST routes, `/events` and
  per-instance console WebSockets (backfill, live stream, slow-client
  resync), TOML config loading/persisting (0600) and token management.
- `packages/mcpanel_daemon/discord.py` — per-instance Discord webhook
  notifier (edit-in-place or new-message, colour-coded status embed with
  recent logs, ANSI/IP/user-regex redaction, 429/5xx handling, debounced
  state-change posts). See `docs/discord.md`.
- `packages/mcpanel_gui` — PySide6 client: node list with one-click local
  node, add/edit/remove remote nodes (URL + token), status panel +
  start/stop/restart, live console (pause-on-scroll-up, command history),
  settings tab (launch/probe/restart/Discord + send test). `ApiClient` is
  Qt-free; tokens live in the OS keyring or a 0600 fallback file; sockets run
  off the UI thread with reconnect/backoff and clear
  offline/unauthorized/incompatible states.
- `docs/remote-access.md` — SSH port-forward (recommended) and reverse proxy
  with TLS (Caddy/nginx), plus verification steps.
- `docs/openapi.json` — OpenAPI schema generated from the daemon.
- `config.example.toml` — the annotated daemon configuration.
- `packaging/mcpanel-daemon.service` — systemd user unit example.
- `LICENSE` — MIT.

`mcpanel-daemon --config config.toml` runs the daemon and `mcpanel-gui` the
client. Known limitation (documented in ARCHITECTURE.md): a server child dies
with its daemon — detached mode is a future option, not built. RCON-based TPS
is config-reserved but not implemented in v0.1.

## Quickstart (daemon + GUI)

```sh
uv sync --all-packages
cp config.example.toml config.toml     # edit the [[instances]] section
uv run mcpanel-daemon --config config.toml
# On first run the API token is printed once and stored in <data_dir>/token (0600).
curl -H "Authorization: Bearer <token>" http://127.0.0.1:8080/api/v1/instances

# In another terminal:
uv run mcpanel-gui
# "Add local node" defaults to http://127.0.0.1:8080; paste the token when asked.
```

## Run the daemon as a systemd user service

```sh
mkdir -p ~/.config/systemd/user
cp packaging/mcpanel-daemon.service ~/.config/systemd/user/
# edit WorkingDirectory / ExecStart to match your paths
systemctl --user daemon-reload
systemctl --user enable --now mcpanel-daemon
journalctl --user -u mcpanel-daemon -f
```

The daemon stops its Minecraft children itself on SIGTERM (the app lifespan
runs the shutdown); the unit's `TimeoutStopSec` covers the slowest stop
escalation.

## Layout

```
packages/
├── mcpanel-common/   # pydantic models: config schema + API contract
├── mcpanel-daemon/   # FastAPI daemon (process manager, poller, Discord)
└── mcpanel-gui/      # PySide6 client (ApiClient, view models, widgets)
tests/                # one pytest run for the whole workspace
docs/                 # openapi.json + guides (remote access, Discord)
```

## Development

Requires Python 3.12+ and [uv](https://docs.astral.sh/uv/).

```sh
uv sync                 # install workspace + dev tools
uv run pytest           # test suite
uv run ruff check       # lint
uv run pyright          # type check (strict for common + daemon)
uv run python -m mcpanel_daemon.openapi > docs/openapi.json   # regenerate schema
```

## Phase checklist

- [x] **0** Architecture, API contract, config schema
- [x] **1** Process manager + log buffer + state machine + fake server
- [x] **2** Status poller and snapshot merging
- [x] **3** REST/WebSocket API + auth
- [x] **4** Discord notifier
- [x] **5** GUI for a single node
- [x] **6** Multi-node/remote support + docs
- [x] **7** Hardening: error paths, graceful shutdown, packaging, examples

## License

MIT. No code is borrowed from the referenced projects (crafty, wings, MCSManager,
msw-go); mcstatus is a dependency under Apache-2.0.
