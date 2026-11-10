# Release notes

## v1.9.3
- Fix orphaned app processes after close: prevent per-instance process-manager runtime threads from blocking forever on exit, harden GUI exit/cleanup flow, and stop the event forwarder when the event bus closes.
- Update project and app license to GNU Affero General Public License v3.0 (AGPL-3.0).
- Refresh app and project logo/icon assets.

## v1.9.2
- Fix RAM (Min/Max) settings being ignored on server start:
  - Always include `-Xms`/`-Xmx` from the configured RAM values (including preset "None").
  - Strip stale `-Xms`/`-Xmx` entries from custom JVM args on load/save so they cannot override the selected RAM (the JVM uses the last occurrence).
  - Parse displayed RAM with last-wins semantics and validate RAM values before saving/creating an instance.
  - Persist daemon config patches to TOML and push the updated config to the running process manager so starts use the new settings.

## v1.9.1
- Clean up repository contents to keep project sources, packaging assets, and user-facing documentation.
- Bump app and crate versions to `1.9.1`.

## v1.9.0
- Fix process manager/runtime issues: valid tokio channel handling, apply updated instance config to the running process manager, and show crash-during-start diagnostics (exit code + log tail).
- Stop local servers/processes on GUI exit so closing the app does not leave Minewrap or managed server processes running in the background.
- Fix Discord webhook embed payload and add best-effort state/status notifications from the GUI.
- Fix Console wiring and log sequencing/backfill so server logs and commands work in the UI.
- UI: add a built-in Close app button and improve spacing/margins.
