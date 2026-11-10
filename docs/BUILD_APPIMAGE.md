# Build the Minewrap AppImage (Linux x86_64)

The **AppImage** is Minewrap's primary product: a single portable file that bundles the desktop app (Tauri + Svelte) and the `minewrapd` daemon sidecar.

Prebuilt releases are published at <https://github.com/longme179/Minewrap/releases>. This document describes how to build the AppImage from source.

## Prerequisites

### System dependencies (build-time)

Tauri links against WebKitGTK and libsoup. Install the **development** packages for:

- `webkit2gtk-4.1` (pkg-config name: `webkit2gtk-4.1`)
- `libsoup-3.0` (pkg-config name: `libsoup-3.0`)
- GTK 3

The build script verifies `webkit2gtk-4.1` and `libsoup-3.0` via `pkg-config` and exits with install hints if they are missing.

**Fedora / RHEL / CentOS Stream**
```bash
sudo dnf install webkit2gtk4.1-devel libsoup3-devel gtk3-devel pkgconf-pkg-config
```

**Debian / Ubuntu** (package names may vary by release; ensure WebKitGTK 4.1 + libsoup3 dev packages are installed)
```bash
sudo apt install libwebkit2gtk-4.1-dev libsoup-3.0-dev libgtk-3-dev pkg-config
```

You also need common build tools (`curl`, a C/C++ toolchain) for Rust crates and bundling.

### Toolchain

- **Rust** (stable) with `cargo` — e.g. via [rustup](https://rustup.rs/) or your distro.
- **Node.js** and **pnpm** — for building the frontend. `@tauri-apps/cli` is a dev dependency in `ui/`.
  ```bash
  npm install -g pnpm   # or: corepack enable
  ```
- **appimagetool** (x86_64) — required to produce the `.AppImage`. If not found on `PATH` or at `tools/appimagetool`, `packaging/build-linux.sh` downloads the official continuous build into `tools/` (gitignored). You can also install it manually and put it on `PATH`.

## Quick build

From the repository root:

```bash
./packaging/build-linux.sh
```

Output:
- `dist/Minewrap-x86_64.AppImage` — portable AppImage (copied from `target/release/bundle/appimage/`)
- `dist/BUILD_FROM_SOURCE.md` — generated record of the build steps (in `dist/`, gitignored)

> Close any running Minewrap AppImage before rebuilding if you copy over `dist/Minewrap-x86_64.AppImage` (overwriting a running AppImage can fail with "Text file busy").

## What the script does

1. Checks `pkg-config` for `webkit2gtk-4.1` and `libsoup-3.0`.
2. Resolves `appimagetool` (PATH, `tools/appimagetool`, or download to `tools/`).
3. Builds the UI: `cd ui && pnpm install --ignore-scripts && pnpm run build`.
4. Builds the daemon: `cargo build --release -p minewrapd`.
5. Stages the daemon sidecar: copies `target/release/minewrapd` to `src-tauri/binaries/minewrapd-x86_64-unknown-linux-gnu` (directory is gitignored; Tauri bundles it via `externalBin`).
6. Bundles the AppImage: runs the Tauri CLI from `ui/node_modules/.bin/tauri` with `build --bundles appimage`.
7. Copies the resulting AppImage to `dist/Minewrap-x86_64.AppImage` and writes `dist/BUILD_FROM_SOURCE.md`.

## Manual build (equivalent)

```bash
# UI
(cd ui && pnpm install && pnpm run build)

# Daemon sidecar for the Tauri bundle
cargo build --release -p minewrapd
mkdir -p src-tauri/binaries
cp target/release/minewrapd src-tauri/binaries/minewrapd-x86_64-unknown-linux-gnu

# AppImage bundle (from repo root)
./ui/node_modules/.bin/tauri build --bundles appimage

# Result
ls target/release/bundle/appimage/*.AppImage
```

## Run the AppImage

```bash
chmod +x dist/Minewrap-x86_64.AppImage
./dist/Minewrap-x86_64.AppImage
```

On systems without FUSE:
```bash
./dist/Minewrap-x86_64.AppImage --appimage-extract-and-run
```

## Reproducibility and repository notes

- `Cargo.lock` and `ui/pnpm-lock.yaml` are committed to pin dependencies for reproducible builds.
- Build outputs and fetched tools are **not** committed: `target/`, `dist/`, `ui/node_modules/`, `src-tauri/binaries/`, `tools/` are gitignored. The build script creates/populates the required build-time artifacts locally.
- Tauri bundle configuration (AppImage target, icons, external daemon binary) lives in `src-tauri/tauri.conf.json`; AppImage packaging assets are in `packaging/`.

## Troubleshooting

- **`webkit2gtk-4.1` / `libsoup-3.0` not found**: install the matching `-dev`/`-devel` packages and ensure `pkg-config` can see them (`pkg-config --modversion webkit2gtk-4.1 libsoup-3.0`).
- **`tauri CLI not found`**: run `cd ui && pnpm install` so `ui/node_modules/.bin/tauri` is available (or install the Tauri CLI and adjust the command).
- **AppImage fails to run (FUSE)**: use `--appimage-extract-and-run`.
- **`Text file busy` when copying to `dist/`**: close the running Minewrap AppImage and rebuild/copy again.

## Related

- [`../README.md`](../README.md) — project overview and quick start
- [`../packaging/build-linux.sh`](../packaging/build-linux.sh) — canonical build script
- [`remote-access.md`](remote-access.md) — accessing a remote `minewrapd` daemon
