#!/usr/bin/env bash
# Build the single-click portable Minewrap AppImage (GUI + daemon).
#
# Linux x86_64. Installs nothing system-wide (nothing even under /root,
# except the one-time appimagetool fetch). When `appimagetool` is
# missing it downloads the official continuous release into ./tools/
# (project-local, gitignored) and puts it on PATH.
#
# Also writes `dist/BUILD_FROM_SOURCE.md` with the exact commands so
# users can reproduce without this script.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
TOOLS_DIR="$ROOT/tools"
TOOLS_BIN="$TOOLS_DIR/appimagetool"

# --- prerequisites (verified, not installed per rule 8) ---
for pkg in webkit2gtk-4.1 libsoup-3.0; do
  if ! pkg-config --exists "$pkg" 2>/dev/null; then
    echo "error: pkg-config package '$pkg' not found." >&2
    echo "install: sudo dnf install webkit2gtk-4.1-devel libsoup-devel gtk3-devel" >&2
    echo "why: Tauri links WebKitGTK; runtime alone will not compile." >&2
    exit 1
  fi
done

# --- resolve / fetch appimagetool ---
resolve_appimagetool() {
  if command -v appimagetool >/dev/null 2>&1; then
    command -v appimagetool
  elif [ -x "$TOOLS_BIN" ]; then
    echo "$TOOLS_BIN"
  else
    mkdir -p "$TOOLS_DIR"
    echo "fetching appimagetool -> $TOOLS_BIN" >&2
    curl -fL -o "$TOOLS_BIN" "$APPIMAGETOOL_URL"
    chmod +x "$TOOLS_BIN"
    echo "$TOOLS_BIN"
  fi
}

APPIMAGETOOL="$(resolve_appimagetool)"
export PATH="$(dirname "$APPIMAGETOOL"):$PATH"
echo "appimagetool: $($APPIMAGETOOL --version 2>&1 | head -1)"

# --- pnpm (UI) ---
if ! command -v pnpm >/dev/null 2>&1; then
  echo "error: pnpm not found on PATH." >&2
  echo "install: npm install -g pnpm  (or enable corepack/pnpm; see README)" >&2
  echo "why: builds the Svelte/Tauri frontend." >&2
  exit 1
fi

# --- build ---
# UI first (vite handles its own deps; pnpm approve-builds may be
# needed once for esbuild — CI does it non-interactively).
if (cd ui && pnpm install --ignore-scripts >/dev/null 2>&1) && \
   (cd ui && pnpm run build >/dev/null 2>&1); then
  echo "ui: built"
else
  echo "error: UI build failed (see 'cd ui && pnpm run build')." >&2
  echo "hint: pnpm approves build scripts once: 'cd ui && pnpm approve-builds' (interactive)." >&2
  exit 1
fi
cargo build --release -p minewrapd
cp "target/release/minewrapd" "src-tauri/binaries/minewrapd-x86_64-unknown-linux-gnu"

# `cargo tauri` needs tauri-cli; use the npm-shipped one from the UI tree.
TAURI="$(cd ui && pnpm exec tauri --version >/dev/null 2>&1 && echo ./ui/node_modules/.bin/tauri || echo "")"
if [ -z "$TAURI" ]; then
  echo "error: tauri CLI not found (run 'cd ui && pnpm install' first)." >&2
  exit 1
fi

# Run from the repo root so tauri-cli finds the project.
"$TAURI" build --bundles appimage 2>&1 | tail -5

BUNDLE_DIR="target/release/bundle/appimage"
ls "$BUNDLE_DIR"/*.AppImage >/dev/null 2>&1 || {
  echo "error: no AppImage found in $BUNDLE_DIR" >&2
  exit 1
}

DEST="dist/Minewrap-x86_64.AppImage"
mkdir -p dist
cp "$BUNDLE_DIR"/*.AppImage "$DEST"
chmod +x "$DEST"

cat > dist/BUILD_FROM_SOURCE.md <<MD
# Build Minewrap v1.9.1 from source (Linux x86_64)

## 1. System dependencies (one-time, system-wide)
\`\`\`bash
sudo dnf install webkit2gtk-4.1-devel libsoup-devel gtk3-devel
# (Tauri links WebKitGTK; runtime alone will not compile)
\`\`\`
Verify: \`pkg-config --exists webkit2gtk-4.1 libsoup-3.0\`

## 2. Toolchain (project-level only)
\`\`\`bash
# Node + pnpm (for the UI)
npm install -g pnpm      # or: corepack enable
# Rust (stable) — system package or rustup, not installed by this repo
cargo --version

# appimagetool (fetched by packaging/build-linux.sh if missing)
curl -fL -o tools/appimagetool https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
chmod +x tools/appimagetool
export PATH="\$PWD/tools:\$PATH"
\`\`\`

## 3. Build
\`\`\`bash
./packaging/build-linux.sh
# -> dist/Minewrap-x86_64.AppImage
\`\`\`
Manual equivalent (what the script runs):
\`\`\`bash
(cd ui && pnpm install && pnpm run build)
cargo build --release -p minewrapd
cp target/release/minewrapd src-tauri/binaries/minewrapd-x86_64-unknown-linux-gnu
./ui/node_modules/.bin/tauri build --bundles appimage
\`\`\`

## 4. Run
\`\`\`bash
./dist/Minewrap-x86_64.AppImage                      # GUI (FUSE)
./dist/Minewrap-x86_64.AppImage --appimage-extract-and-run  # FUSE-less hosts
./dist/Minewrap-x86_64.AppImage --daemon --config minewrap.toml  # daemon only
\`\`\`
MD

echo "built: $DEST"
echo "reproduce: dist/BUILD_FROM_SOURCE.md"
