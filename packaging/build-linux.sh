#!/usr/bin/env bash
# Build Linux artifacts:
#   dist/mcpanel-daemon                  single-file daemon binary
#   dist/mcpanel-gui/                    one-folder GUI (portable directory)
#   dist/mcpanel-gui-x86_64.AppImage     one-file desktop app
#
# Requirements: Linux x86_64, curl (for appimagetool), FUSE optional
# (appimagetool is run with --appimage-extract-and-run).
#
# Cross-compiling is not supported: run this on the target OS. The PyInstaller
# specs are platform-neutral, so the same commands produce .exe on Windows and
# a .app on macOS when run there (the AppImage step is Linux-only).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

DIST="$ROOT/dist"
BUILD="$ROOT/build"
mkdir -p "$DIST" "$BUILD"

echo "==> syncing workspace"
uv sync --all-packages

echo "==> building daemon binary (onefile)"
uv run pyinstaller \
  --noconfirm --clean \
  --distpath "$DIST" \
  --workpath "$BUILD/pyinstaller" \
  "$ROOT/packaging/pyinstaller/mcpanel-daemon.spec"

echo "==> building GUI (one-folder)"
uv run pyinstaller \
  --noconfirm --clean \
  --distpath "$DIST" \
  --workpath "$BUILD/pyinstaller" \
  "$ROOT/packaging/pyinstaller/mcpanel-gui.spec"

echo "==> assembling AppDir"
APPDIR="$DIST/mcpanel-gui.AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/lib" "$APPDIR/usr/bin" "$APPDIR/usr/share/icons/hicolor/256x256/apps"
cp -r "$DIST/mcpanel-gui" "$APPDIR/usr/lib/mcpanel-gui"
cp "$DIST/mcpanel-daemon" "$APPDIR/usr/bin/mcpanel-daemon"
cp "$ROOT/packaging/appimage/AppRun" "$APPDIR/AppRun"
chmod +x "$APPDIR/AppRun"
cp "$ROOT/packaging/appimage/mcpanel.desktop" "$APPDIR/mcpanel.desktop"
uv run python "$ROOT/packaging/make_icon.py" "$APPDIR/mcpanel.png"
cp "$APPDIR/mcpanel.png" "$APPDIR/.DirIcon"
cp "$APPDIR/mcpanel.png" "$APPDIR/usr/share/icons/hicolor/256x256/apps/mcpanel.png"

echo "==> fetching appimagetool"
TOOL="$BUILD/appimagetool-x86_64.AppImage"
if [ ! -x "$TOOL" ]; then
  curl -L --fail --silent --show-error \
    -o "$TOOL" \
    "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
  chmod +x "$TOOL"
fi

echo "==> building AppImage"
ARCH=x86_64 "$TOOL" --appimage-extract-and-run "$APPDIR" "$DIST/mcpanel-gui-x86_64.AppImage"
echo
echo "Done:"
du -sh "$DIST/mcpanel-daemon" "$DIST/mcpanel-gui" "$DIST/mcpanel-gui-x86_64.AppImage" 2>/dev/null || true
