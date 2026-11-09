# Minewrap v2

Fast, light desktop manager for Minecraft Java Edition servers (one or many, local or remote). A Tauri 2 + Svelte 5 app over a Tauri-free Rust core; the same core runs headless as `minewrapd` (axum REST+WebSocket) for always-on remote nodes and Discord posts.

**v2.0.0** — Rust workspace + Svelte UI. v1.0.0 (Python) is in `legacy/` (tag `v1.0.0`), kept runnable; parity audit in `docs/PARITY.md`.

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
Cargo.toml           # workspace
crates/minewrap-core # Tauri-free core: config, process, logs, poller, discord, properties, mods
crates/minewrapd     # headless axum daemon (REST + WS)
crates/minewrap-remote # reqwest client for remote nodes
crates/fake-mc-server # stress fixture
src-tauri/           # Tauri commands/events/capabilities
ui/                  # Vite + Svelte + TS SPA
docs/                # PARITY, design, remote-access, adrs
design-system/       # ui-ux-pro-max generated tokens
packaging/           # systemd user unit
scripts/             # measure-rss.sh
legacy/              # v1.0.0 (Python, runnable)
```

## Tests / lints

```bash
cargo test --workspace
cd ui && pnpm test && pnpm run check && pnpm build
```

Warnings are denied in the Rust workspace (`[workspace.lints.rust]`).

## Performance

Budgets and how to measure them: [`scripts/measure-rss.sh`](scripts/measure-rss.sh) (main Tauri process, each WebKit helper, and the total, separately). Real heavy-modpack test (NeoForge 1.21.1, 6–8 GB): run the server, open the console, then:

```bash
cargo run -p minewrap-tauri
./scripts/measure-rss.sh <tauri-main-pid> 30
```

Pass criterion: main Tauri process stable under 30 MB; otherwise the report lists the biggest contributors honestly.

## Docs

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — IPC contract + config schema + threading/RSS budget
- [`docs/PARITY.md`](docs/PARITY.md) — v1 → v2 parity audit
- [`docs/design/BRIEF.md`](docs/design/BRIEF.md) + [`DECISIONS.md`](docs/design/DECISIONS.md) — design evidence
- [`docs/remote-access.md`](docs/remote-access.md) — SSH forward (recommended)
- [`docs/adr/`](docs/adr/) — architecture decision records

## Release notes

See [`docs/RELEASE_NOTES.md`](docs/RELEASE_NOTES.md).

## License

MIT — see [LICENSE](LICENSE).
