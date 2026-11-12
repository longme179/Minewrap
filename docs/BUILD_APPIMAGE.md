# Building the Minewrap AppImage

The AppImage (`dist/Minewrap-x86_64.AppImage`) is Minewrap's primary
distributable: a single portable file that bundles the pure-Rust Slint
GUI (`crates/minewrap-gui`) and the `minewrapd` daemon sidecar.

The GUI and the daemon sit side-by-side in the image
(`usr/bin/minewrap-gui` + `usr/bin/minewrapd`), so the GUI's daemon
locator finds `minewrapd` next to the executable without needing
`MINEWRAPD_BIN`. `--daemon` on the AppRun forwards to `minewrapd`.

No WebKitGTK / libsoup or JavaScript-frontend toolchain is involved.

## Prerequisites (one-time, system-wide)

```bash
sudo dnf install fontconfig-devel freetype-devel
# Slint's software renderer uses fontconfig/freetype for text; cargo links them.
```

Verify:
```bash
pkg-config --exists fontconfig freetype2 && cargo --version
```

## Build

```bash
./packaging/build-linux.sh
# -> dist/Minewrap-x86_64.AppImage
```

The script:
1. builds `cargo build --release -p minewrapd` and
   `cargo build --release -p minewrap-gui --no-default-features`
   (accessibility OFF = measured minimum-RAM configuration),
2. stages an AppDir (`usr/bin/minewrap-gui`, `usr/bin/minewrapd`,
   `usr/share/applications/minewrap.desktop` with `Exec=minewrap-gui`,
   the icon, and `AppRun` that execs the GUI),
3. bundles it with `appimagetool` (fetched into `tools/` on first run
   if missing),
4. writes `dist/BUILD_FROM_SOURCE.md` with the exact reproduction steps.

## Run

```bash
./dist/Minewrap-x86_64.AppImage                      # GUI (FUSE)
./dist/Minewrap-x86_64.AppImage --appimage-extract-and-run  # FUSE-less hosts
./dist/Minewrap-x86_64.AppImage --daemon --config minewrap.toml  # daemon only
```

See `ARCHITECTURE.md` §Packaging and `dist/BUILD_FROM_SOURCE.md` for
details.
