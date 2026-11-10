# Release notes

## v1.9.1
- Clean up repository contents to keep project sources, packaging assets, and user-facing documentation.
- Bump app and crate versions to `1.9.1`.

## v1.9.0
- Fix process manager/runtime issues: valid tokio channel handling, apply updated instance config to the running process manager, and show crash-during-start diagnostics (exit code + log tail).
- Stop local servers/processes on GUI exit so closing the app does not leave Minewrap or managed server processes running in the background.
- Fix Discord webhook embed payload and add best-effort state/status notifications from the GUI.
- Fix Console wiring and log sequencing/backfill so server logs and commands work in the UI.
- UI: add a built-in Close app button and improve spacing/margins.
