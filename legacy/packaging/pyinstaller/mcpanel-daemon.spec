# PyInstaller spec: single-file daemon binary.
# Build with: packaging/build-linux.sh (or: uv run pyinstaller this-file.spec)

import os

from PyInstaller.utils.hooks import collect_submodules

ROOT = os.path.abspath(os.path.join(SPECPATH, "..", ".."))

a = Analysis(
    [os.path.join(ROOT, "packaging", "entrypoints", "daemon_entry.py")],
    pathex=[
        os.path.join(ROOT, "packages", "mcpanel-daemon", "src"),
        os.path.join(ROOT, "packages", "mcpanel-common", "src"),
    ],
    binaries=[],
    datas=[],
    hiddenimports=collect_submodules("uvicorn"),
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=["tkinter"],
    noarchive=False,
)

pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    [],
    name="mcpanel-daemon",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    upx_exclude=[],
    runtime_tmpdir=None,
    console=True,
)
