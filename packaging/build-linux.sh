#!/usr/bin/env bash
# Build the single-click portable Minewrap AppImage (Slint GUI + daemon).
#
# Linux x86_64. Installs nothing system-wide (nothing even under /root,
# except the one-time appimagetool fetch). When `appimagetool` is missing
# it downloads the official continuous release into ./tools/
# (project-local, gitignored) and puts it on PATH.
#
# The GUI is the pure-Rust Slint client (crates/minewrap-gui), built with
# --no-default-features so accessibility is OFF (measured minimum RAM
# configuration, see crates/minewrap-gui/Cargo.toml feature comment).
# The daemon (minewrapd) is placed beside the GUI binary in usr/bin/ so
# the GUI's locate_daemon_binary finds it without MINEWRAPD_BIN.
#
# No web GUI toolkit / JavaScript frontend is involved — Slint's software
# renderer needs no system GUI library beyond fontconfig/freetype, which
# cargo links itself.
#
# Also writes `dist/BUILD_FROM_SOURCE.md` with the exact commands so
# users can reproduce without this script.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
TOOLS_DIR="$ROOT/tools"
TOOLS_BIN="$TOOLS_DIR/appimagetool"

APP_NAME="Minewrap"
APP_DIR="$ROOT/$APP_NAME.AppDir"
DEST="dist/Minewrap-x86_64.AppImage"

# --- prerequisites: cargo + Slint GUI libs (no web GUI toolkit / JS frontend) ---
if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo not found on PATH." >&2
  echo "install: https://rustup.rs" >&2
  exit 1
fi
for pkg in fontconfig freetype2; do
  if ! pkg-config --exists "$pkg" 2>/dev/null; then
    echo "error: pkg-config package '$pkg' not found." >&2
    echo "install: sudo dnf install fontconfig-devel freetype-devel" >&2
    echo "why: Slint software renderer uses fontconfig/freetype for text (cargo links them)." >&2
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
export ARCH="${ARCH:-x86_64}"
echo "appimagetool: $($APPIMAGETOOL --version 2>&1 | head -1)"

# --- build daemon + Slint GUI (release) ---
echo "building minewrapd (cargo release)…"
cargo build --release -p minewrapd
echo "building minewrap-gui (cargo release --no-default-features)…"
cargo build --release -p minewrap-gui --no-default-features

# --- stage AppDir ---
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/usr/bin" \
         "$APP_DIR/usr/share/applications" \
         "$APP_DIR/usr/share/icons/hicolor/512x512/apps"

install -Dm755 "target/release/minewrap-gui" "$APP_DIR/usr/bin/minewrap-gui"
install -Dm755 "target/release/minewrapd"   "$APP_DIR/usr/bin/minewrapd"

# desktop file (Exec=minewrap-gui so the GUI launches from the menu)
sed -e 's/^Exec=.*/Exec=minewrap-gui/' \
    packaging/appimage/minewrap.desktop > "$APP_DIR/usr/share/applications/minewrap.desktop"
# appimagetool also reads the desktop file from the AppDir root
cp "$APP_DIR/usr/share/applications/minewrap.desktop" "$APP_DIR/minewrap.desktop"

# icon (512x512 apps + top-level minewrap.png for appimagetool detection)
cp packaging/appimage/minewrap.png "$APP_DIR/usr/share/icons/hicolor/512x512/apps/minewrap.png"
cp packaging/appimage/minewrap.png "$APP_DIR/minewrap.png"

# AppRun
cat > "$APP_DIR/AppRun" <<'EOF'
#!/bin/sh
# AppImage entry point.
#
#   Minewrap-x86_64.AppImage                        -> GUI (Slint)
#   Minewrap-x86_64.AppImage --daemon --config ...  -> bundled daemon
#   Minewrap-x86_64.AppImage --daemon --help        -> daemon usage
HERE="$(dirname "$(readlink -f "$0")")"

if [ "${1:-}" = "--daemon" ]; then
  shift
  exec "$HERE/usr/bin/minewrapd" "$@"
fi
exec "$HERE/usr/bin/minewrap-gui" "$@"
EOF
chmod +x "$APP_DIR/AppRun"

# --- bundle with appimagetool ---
mkdir -p dist
echo "bundling AppImage with appimagetool…"
"$APPIMAGETOOL" "$APP_DIR" "$DEST" 2>&1 | tail -8
chmod +x "$DEST"

echo "built: $DEST"

cat > dist/BUILD_FROM_SOURCE.md <<MD
# Build Minewrap v1.10.0 from source (Linux x86_64)

The GUI is the pure-Rust Slint client (\`crates/minewrap-gui\`). The daemon
(\`crates/minewrapd\`) runs headless. No web GUI toolkit is involved.

## 1. System dependencies (one-time, system-wide)
\`\`\`bash
sudo dnf install fontconfig-devel freetype-devel
# (Slint software renderer uses fontconfig/freetype for text; cargo links them)
\`\`\`
Verify: \`pkg-config --exists fontconfig freetype2\`

## 2. Toolchain (project-level only)
\`\`\`bash
# Rust (stable) — system package or rustup, not installed by this repo
cargo --version

# appimagetool (fetched by packaging/build-linux.sh if missing)
curl -fL -o tools/appimagetool https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
chmod +x tools/appimagetool
export PATH="\$PWD/tools:\$PATH"
export ARCH=x86_64
\`\`\`

## 3. Build
\`\`\`bash
# Daemon + Slint GUI (release; --no-default-features = accessibility OFF,
# the measured minimum-RAM configuration)
cargo build --release -p minewrapd
cargo build --release -p minewrap-gui --no-default-features

# AppImage (packaging/build-linux.sh stages the AppDir and runs appimagetool)
./packaging/build-linux.sh
# -> dist/Minewrap-x86_64.AppImage
\`\`\`

## 4. Run
\`\`\`bash
./dist/Minewrap-x86_64.AppImage                      # GUI (FUSE)
./dist/Minewrap-x86_64.AppImage --appimage-extract-and-run  # FUSE-less hosts
./dist/Minewrap-x86_64.AppImage --daemon --config minewrap.toml  # daemon only
\`\`\`

The daemon binary sits beside the GUI in the AppImage (\`usr/bin/minewrapd\`),
so the GUI's daemon locator finds it without setting MINEWRAPD_BIN.
MD

echo "reproduce: dist/BUILD_FROM_SOURCE.md"
