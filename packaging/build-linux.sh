#!/usr/bin/env bash
# Build the single-click portable Minewrap AppImage (GUI + daemon).
#
# Linux x86_64 first. Installs nothing system-wide; if `appimagetool`
# is missing it prints the exact DNF command and exits.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# --- prerequisites (verified, not installed per rule 8) ---
for pkg in webkit2gtk-4.1 libsoup-3.0; do
  if ! pkg-config --exists "$pkg" 2>/dev/null; then
    echo "error: pkg-config package '$pkg' not found." >&2
    echo "install: sudo dnf install webkit2gtk-4.1-devel libsoup-devel gtk3-devel" >&2
    echo "why: Tauri links WebKitGTK; runtime alone will not compile." >&2
    exit 1
  fi
done

if ! command -v appimagetool >/dev/null 2>&1; then
  echo "error: appimagetool not found on PATH." >&2
  echo "install: sudo dnf install appimagetool" >&2
  echo "why: assembles the fusesquashfs AppImage from the Tauri bundle." >&2
  echo "alternative: download the static appimagetool release into ./tools/ and put it on PATH." >&2
  exit 1
fi

# --- build ---
(cd ui && pnpm install && pnpm run build)
cargo build --release -p minewrapd
cp "target/release/minewrapd" "src-tauri/binaries/minewrapd-x86_64-unknown-linux-gnu"

cargo tauri build --target x86_64-unknown-linux-gnu

BUNDLE_DIR="src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/appimage"
ls "$BUNDLE_DIR"/*.AppImage >/dev/null 2>&1 || {
  echo "error: no AppImage found in $BUNDLE_DIR" >&2
  exit 1
}

# --- assemble portable single-file ---
DEST="dist/Minewrap-x86_64.AppImage"
mkdir -p dist
cp "$BUNDLE_DIR"/*.AppImage "$DEST"
chmod +x "$DEST"
echo "built: $DEST (double click, or: $DEST --appimage-extract-and-run on FUSE-less hosts)"
