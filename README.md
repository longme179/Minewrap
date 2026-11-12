<p align="center">
  <img src="docs/assets/logo.svg" width="128" alt="Minewrap logo">
</p>

# Minewrap

Fast, light desktop manager for Minecraft Java Edition servers (one or many, local or remote). A pure-Rust Slint GUI over a headless Rust daemon: since **v1.10.0** `minewrapd` (axum REST+WebSocket) is the always-on half — servers, SLP status polling and Discord posts keep running with the window closed — and the GUI is a pure client that auto-spawns it on demand.

**AppImage (Linux x86_64) is the primary distribution.** Prebuilt AppImages are published on [GitHub Releases](https://github.com/longme179/Minewrap/releases).

## Screenshots

| Console | Settings |
|:---:|:---:|
| <img src="docs/screenshots/console-offline.png" width="440" alt="Console tab with server offline"><br>**Console** — live server logs, filtering, and command input | <img src="docs/screenshots/settings-launch.png" width="440" alt="Settings tab with launch configuration"><br>**Settings** — launch mode, Java runtime, RAM, probe, restart policy |

| Properties | Mods |
|:---:|:---:|
| <img src="docs/screenshots/server-properties.png" width="440" alt="Properties tab showing raw server.properties"><br>**Properties** — edit `server.properties` directly, preserving comments and order | <img src="docs/screenshots/mods.png" width="440" alt="Mods tab listing installed mods"><br>**Mods** — search installed mods, see sizes, enable/disable |

| Notifications | Console (server online) |
|:---:|:---:|
| <img src="docs/screenshots/notifications-discord.png" width="440" alt="Notifications tab with Discord webhook settings"><br>**Notifications** — Discord webhook status posts with write-only URL storage | <img src="docs/screenshots/console-online.png" width="440" alt="Console tab streaming live server logs"><br>**Console** — log stream backfill and live tail while the server runs |

## Quick start

### Run the AppImage
```bash
chmod +x Minewrap-x86_64.AppImage
./Minewrap-x86_64.AppImage
# On systems without FUSE:
./Minewrap-x86_64.AppImage --appimage-extract-and-run
```

### Build everything from source
```bash
./build.sh              # daemon + GUI AppImage (installs missing system deps via sudo)
./build.sh --daemon-only  # headless host: just minewrapd
```

### Run from source (development)
```bash
# Run the Slint GUI (development; --no-default-features = accessibility OFF,
# the measured minimum-RAM configuration)
cargo run -p minewrap-gui --no-default-features

# Run the headless daemon (binds 127.0.0.1:8080 by default)
cargo run -p minewrapd -- --config examples/minewrap.toml
```

> **v1.10.0:** closing the GUI no longer stops servers. The GUI
> auto-spawns `minewrapd` and only disconnects on exit. To stop
> everything: `systemctl --user stop minewrapd` (or `kill -TERM` the
> daemon) — it stops every instance gracefully, then exits.

See [`docs/BUILD_APPIMAGE.md`](docs/BUILD_APPIMAGE.md) for full AppImage build prerequisites and steps, and [`docs/remote-access.md`](docs/remote-access.md) for reaching a remote daemon over SSH port-forward.

## Repo layout

```
build.sh               # build-from-source script (daemon-only or full AppImage)
Cargo.toml             # Rust workspace
Cargo.lock             # locked Rust dependencies (reproducible builds)
crates/minewrap-core   # core: config, process, logs, poller, discord, properties, mods
crates/minewrapd       # headless axum daemon (REST + WS): state keeper, SLP poller, Discord service
crates/minewrap-gui    # pure-Rust Slint GUI client (auto-spawns minewrapd)
crates/minewrap-remote # reqwest + WS client + shared DTOs (used by the GUI)
packaging/             # AppImage build script, AppRun/desktop, systemd unit
examples/              # example config
docs/                  # documentation
```

## Build checks

```bash
cargo build --workspace
cargo build -p minewrap-gui --no-default-features
```

## Docs

- [`docs/BUILD_APPIMAGE.md`](docs/BUILD_APPIMAGE.md) — build the AppImage (main product)
- [`ARCHITECTURE.md`](ARCHITECTURE.md) — architecture and config overview
- [`docs/remote-access.md`](docs/remote-access.md) — reaching a remote daemon
- [`docs/RELEASE_NOTES.md`](docs/RELEASE_NOTES.md) — release notes

## License

GNU Affero General Public License v3.0 (AGPL-3.0) — see [LICENSE](LICENSE).
