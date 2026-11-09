# Minewrap

**Run and monitor Minecraft Java Edition servers from a desktop app — on this
machine or a remote one — without keeping a terminal open or exposing RCON.**

[![License: MIT](LICENSE)](LICENSE)
[![Rust & Svelte stack](ARCHITECTURE.md)](ARCHITECTURE.md)
[![Platform: Linux x86_64](ARCHITECTURE.md)](ARCHITECTURE.md)

> **Status:** v2 (Tauri + Svelte) is in development feedstock. The last stable
> release is **[v1.0.0](https://github.com/longme179/Minewrap/releases/tag/v1.0.0)**
> — the Python/PySide6 build, kept runnable under [`legacy/`](legacy/README.md)
> (tagged `v1.0.0`). A single all-in-one **AppImage** contains GUI + daemon.

## Download (v1.0.0)

```
mcpanel-x86_64.AppImage      (~93 MB, GUI + daemon in one file)
mcpanel-x86_64.AppImage.sha256
```

Requirements: Linux x86_64. Direct execution needs FUSE; on FUSE-less systems
run it with `--appimage-extract-and-run`.

```sh
chmod +x mcpanel-x86_64.AppImage
./mcpanel-x86_64.AppImage
```

## Quick start (v1.0.0)

1. Launch the AppImage — the setup wizard opens automatically.
2. Choose your server directory and server jar, set JVM args (e.g. `-Xmx4G`),
   optionally add a Discord webhook, then **Create & start**.
3. The wizard writes `~/mcpanel/config.toml`, starts the bundled daemon in the
   background (detached — it survives closing the GUI), stores the API token
   (`0600`) and connects the node.
4. Click the instance: **Console** for logs and commands (`stop`, `list`, …),
   **Settings** for launch/probe/restart and Discord.

> mcpanel starts and owns its own server process. A server started by hand in
> a terminal is invisible to it — stop that first; two servers in the same
> world/port will conflict.

## What v1.0.0 does

- start/stop/restart with an explicit state machine, startup timeout,
  graceful `stop` → SIGTERM → SIGKILL escalation
- live console with backfill, command input and history, pause-on-scroll
- status probe (players, version, MOTD, latency, uptime) with a separate
  reachability state (`unresponsive` instead of a false `offline`)
- crash-restart policy with backoff and restart-loop protection
- Discord: one message edited in place every N seconds (default 10 min) plus
  debounced state-change posts; ANSI/IP/user-regex redaction
- multi-node GUI (local + remote by token, SSH port-forward recommended or
  TLS reverse proxy), bearer-token auth, loopback bind by default

## This repository

| Path | What |
|---|---|
| `legacy/` | v1.0.0 (Python, runnable) — parity reference |
| `docs/PARITY.md` | full feature/API/test audit of v1.0.0 with v2 targets |
| `ARCHITECTURE.md` | v2 architecture, IPC contract, config schema |
| `docs/design/` | design brief + ui-ux-pro-max decisions |
| `design-system/` | persisted ui-ux-pro-max design system (skill output) |
| `docs/screenshot.png` | v1.0.0 GUI screenshot |

The v2 stack (Tauri 2 + Svelte 5 + Rust workspace) is being built on `main`
starting with `minewrap-core`. See `ARCHITECTURE.md` for the plan and layout.

## License

MIT — see [LICENSE](LICENSE).
