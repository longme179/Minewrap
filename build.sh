#!/usr/bin/env bash
# Minewrap — build from source (v1.10.0).
#
# Modes:
#   ./build.sh               # everything: daemon + GUI (embedded frontend)
#   ./build.sh --daemon-only # headless daemon only (no GUI deps needed)
#
# The script installs missing system dependencies where it can (needs
# root via sudo for the package manager step; skip with MINEWRAP_NO_SUDO=1
# if the deps are already present) and then builds with the project's
# pinned toolchains: cargo (workspace, release) for the Rust side and pnpm
# for the Svelte UI.
#
# Outputs:
#   binaries/minewrapd            — standalone daemon
#   target/release/minewrap-tauri — GUI (frontend embedded at compile time)
#
# AppImage bundling is intentionally NOT part of this script yet — it moves
# to the P1 packaging step together with the native GUI switch.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DAEMON_ONLY=0
[[ "${1:-}" == "--daemon-only" ]] && DAEMON_ONLY=1

log() { printf '[build] %s\n' "$*"; }
die() { printf '[build] %s\n' "$*" >&2; exit 1; }

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

log "building the Svelte UI (fills ui/dist — embedded at compile time)…"
(cd "$ROOT/ui" && pnpm build)

TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
log "staging daemon sidecar (src-tauri/binaries/minewrapd-$TRIPLE)…"
install -Dm755 target/release/minewrapd "$ROOT/src-tauri/binaries/minewrapd-$TRIPLE"

log "building the GUI (cargo release — embeds ui/dist + sidecar)…"
cargo build --release -p minewrap-tauri --features custom-protocol

log "done. daemon: binaries/minewrapd — GUI: target/release/minewrap-tauri"
log "(AppImage bundling arrives with the P1 native-GUI packaging step.)"
