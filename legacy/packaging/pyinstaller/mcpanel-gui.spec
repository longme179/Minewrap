# PyInstaller spec: one-folder GUI build (wrapped into an AppImage).
# Build with: packaging/build-linux.sh (or: uv run pyinstaller this-file.spec)

import os

from PyInstaller.utils.hooks import collect_submodules, copy_metadata

ROOT = os.path.abspath(os.path.join(SPECPATH, "..", ".."))

datas = []
for package in ("keyring", "platformdirs"):
    datas += copy_metadata(package)

a = Analysis(
    [os.path.join(ROOT, "packaging", "entrypoints", "gui_entry.py")],
    pathex=[
        os.path.join(ROOT, "packages", "mcpanel-gui", "src"),
        os.path.join(ROOT, "packages", "mcpanel-common", "src"),
    ],
    binaries=[],
    datas=datas,
    hiddenimports=collect_submodules("keyring.backends"),
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=["tkinter", "PySide6.QtWebEngineCore", "PySide6.QtWebEngineWidgets"],
    noarchive=False,
)

pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name="mcpanel-gui",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    console=False,
)

coll = COLLECT(
    exe,
    a.binaries,
    a.datas,
    strip=False,
    upx=False,
    upx_exclude=[],
    name="mcpanel-gui",
)
