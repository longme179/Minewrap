#!/usr/bin/env bash
# Minewrap — build from source (v1.10.0).
#
# Modes:
#   ./build.sh              # everything: daemon + GUI AppImage
#   ./build.sh --daemon-only# headless daemon only (no GUI deps needed)
#
# The script installs missing system dependencies where it can (needs
# root via sudo for the package manager step; skip with
# MINEWRAP_NO_SUDO=1 if the deps are already present) and then builds
# with the project's pinned toolchains: cargo (workspace, release) for
# the Rust side and pnpm for the Svelte UI.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DAEMON_ONLY=0
[[ "${1:-}" == "--daemon-only" ]] && DAEMON_ONLY=1

log() { printf '\033[1;34m[build]\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m[build]\033[0m %s\n' "$*" >&2; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || die "missing tool: $1 ($2)"; }

# --- toolchains -----------------------------------------------------------
need cargo "install Rust via https://rustup.rs"

if [[ $DAEMON_ONLY -eq 0 ]]; then
  need pnpm "npm i -g pnpm (or corepack enable)"
fi

# --- system deps (GUI only, Linux + webkit2gtk 4.1) -----------------------
if [[ $DAEMON_ONLY -eq 0 && "$(uname -s)" == "Linux" && -z "${MINEWRAP_NO_SUDO:-}" ]]; then
  if ! pkg-config --exists javascriptcoregtk-4.1 2>/dev/null; then
    log "installing webkit2gtk / libsoup build deps (sudo)…"
    if command -v apt-get >/dev/null 2>&1; then
      sudo apt-get update
      sudo apt-get install -y \
        libwebkit2gtk-4.1-dev build-essential curl wget file \
        libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
    elif command -v dnf >/dev/null 2>&1; then
      sudo dnf install -y \
        webkit2gtk4.1-devel openssl-devel librsvg2-devel \
        libappindicator-gtk3-devel xdotool
    else
      log "unknown package manager — install webkit2gtk 4.1 dev packages manually"
    fi
  fi
fi

# --- daemon (always) -------------------------------------------------------
log "building minewrapd (cargo release)…"
cargo build --release -p minewrapd
install -Dm755 target/release/minewrapd "$ROOT/binaries/minewrapd"
log "daemon at binaries/minewrapd"

if [[ $DAEMON_ONLY -eq 1 ]]; then
  log "done (--daemon-only)."
  exit 0
fi

# --- UI + GUI --------------------------------------------------------------
log "installing UI deps (pnpm)…"
(cd "$ROOT/ui" && pnpm install)

log "building the Tauri AppImage (bundles binaries/minewrapd as a sidecar)…"
(cd "$ROOT/ui" && pnpm tauri build)

log "done. AppImage: ui/src-tauri/target/release/bundle/appimage/"
