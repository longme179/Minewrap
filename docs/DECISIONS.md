# Decisions

One line each: decision + reason.

- When the Tauri shell/config image is ambiguous, reuse `minewrap-core::config::instance_view/patch_instance` as the single source for the UI instead of forking a parallel JSON model.
- UI errors never call `String(e)`; they go through one `formatError()` that unwraps the structured `{code, message}` envelope.
- The Settings tabs and pickers will share one `Select` component; native `<select>` is rejected because it breaks the dark theme and cannot prove 4.5:1 contrast.
- Path confinement wins over convenience: any UI path is canonicalized and checked against the instance work dir before use.
- `appimagetool` is fetched by `packaging/build-linux.sh` (project-local ./tools, gitignored); nothing is installed system-wide.
