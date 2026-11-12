#!/usr/bin/env bash
# Minewrap — build from source (v1.10.0).
#
# Modes:
#   ./build.sh               # everything: daemon + Slint GUI
#   ./build.sh --daemon-only # headless daemon only (no GUI needed)
#
# The script installs missing system dependencies where it can (needs
# root via sudo for the package manager step; skip with MINEWRAP_NO_SUDO=1
# if the deps are already present) and then builds with the project's
# pinned toolchain: cargo (workspace, release) for both the daemon and
# the pure-Rust Slint GUI.
#
# Outputs:
#   binaries/minewrapd         — standalone daemon
#   target/release/minewrap-gui — Slint GUI
#
# AppImage bundling lives in packaging/build-linux.sh (stages an AppDir
# from target/release/minewrap-gui + target/release/minewrapd and runs
# appimagetool -> dist/Minewrap-x86_64.AppImage).

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DAEMON_ONLY=0
[[ "${1:-}" == "--daemon-only" ]] && DAEMON_ONLY=1

log() { printf '[build] %s\n' "$*"; }
die() { printf '[build] %s\n' "$*" >&2; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || die "missing tool: $1 ($2)"; }

# --- toolchain -----------------------------------------------------------
need cargo "install Rust via https://rustup.rs"

# --- system deps (GUI: Slint software renderer needs fontconfig/freetype) ---
if [[ $DAEMON_ONLY -eq 0 && "$(uname -s)" == "Linux" && -z "${MINEWRAP_NO_SUDO:-}" ]]; then
  if ! pkg-config --exists fontconfig freetype2 2>/dev/null; then
    log "installing fontconfig/freetype build deps (sudo)…"
    if command -v apt-get >/dev/null 2>&1; then
      sudo apt-get update
      sudo apt-get install -y \
        build-essential curl wget file libxdo-dev libssl-dev \
        libayatana-appindicator3-dev librsvg2-dev \
        libfontconfig1-dev libfreetype6-dev
    elif command -v dnf >/dev/null 2>&1; then
      sudo dnf install -y \
        fontconfig-devel freetype-devel openssl-devel librsvg2-devel \
        libappindicator-gtk3-devel xdotool
    else
      log "unknown package manager — install fontconfig/freetype dev packages manually"
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

# --- GUI (Slint, pure Rust) ----------------------------------------------
log "building minewrap-gui (cargo release --no-default-features, accessibility OFF = measured minimum RAM)…"
cargo build --release -p minewrap-gui --no-default-features

# Default-features build (accessibility ON) is also supported for users who
# need it: cargo build --release -p minewrap-gui

log "done. daemon: binaries/minewrapd — GUI: target/release/minewrap-gui"
log "AppImage: ./packaging/build-linux.sh -> dist/Minewrap-x86_64.AppImage"
