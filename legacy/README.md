# v1.0.0 (Python) — archived

This directory holds the complete, runnable Python/PySide6 v1.0.0 build of
Minewrap, preserved so the rewrite can be measured against it. Tagged
`v1.0.0` in git. Do not modify anything here.

Run it (requires Python 3.12+ and uv, or the existing `uv` venv at repo root
was built with the workspace lockfile at this directory during development):

```sh
cd legacy
uv sync --all-packages
uv run mcpanel-daemon --config ../config.toml     # or its own config
uv run mcpanel-gui
```

The parity target for the Rust/Svelte rewrite is the audit in
`../docs/PARITY.md`. New v2 development lives at the repo root
(`crates/`, `src-tauri/`, `ui/`).
