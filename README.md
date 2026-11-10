# Minewrap v2

Fast, light desktop manager for Minecraft Java Edition servers (one or many, local or remote). A Tauri 2 + Svelte 5 app over a Tauri-free Rust core; the same core runs headless as `minewrapd` (axum REST+WebSocket) for always-on remote nodes and Discord posts.

Current source tree targets the v2 (Rust + Svelte) application. See `docs/PARITY.md` and `docs/RELEASE_NOTES.md` for details.

## Quick start

```bash
# Build the UI
cd ui && pnpm install && pnpm build

# Run the daemon (headless, binds 127.0.0.1:8080 by default)
cargo run -p minewrapd -- --config examples/minewrap.toml

# Run the desktop app (Linux/Wayland first; needs webkit2gtk dev headers:
#   sudo dnf install webkit2gtk-4.1-devel libsoup-devel gtk3-devel)
cargo run -p minewrap-tauri
```

See [`docs/remote-access.md`](docs/remote-access.md) for reaching a remote daemon over SSH port-forward.

## Repo layout

```
Cargo.toml             # workspace
crates/minewrap-core   # Tauri-free core: config, process, logs, poller, discord, properties, mods
crates/minewrapd       # headless axum daemon (REST + WS)
crates/minewrap-remote # reqwest client for remote nodes
src-tauri/             # Tauri desktop app
ui/                    # Vite + Svelte + TypeScript frontend
packaging/             # AppImage/systemd packaging assets
examples/              # example config
docs/                  # documentation
```

## Build checks

```bash
cargo build --workspace
cd ui && pnpm run check && pnpm build
```

## Performance

Target: main Tauri process stable under 30 MB under load (WebKit helpers reported separately). Validate on a heavy modpack by running the desktop app with the server console open and sampling RSS per process (for example with `ps`/`pmap`). Report main Tauri, each WebKit helper, and total separately.

## Docs

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — architecture and config overview
- [`docs/remote-access.md`](docs/remote-access.md) — reaching a remote daemon
- [`docs/RELEASE_NOTES.md`](docs/RELEASE_NOTES.md) — release notes

## License

MIT — see [LICENSE](LICENSE).
