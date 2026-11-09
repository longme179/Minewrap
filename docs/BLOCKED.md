# Minewrap — Blocked

## B-1: Manual click-through (needed from user)
What: a full manual pass through every screen (Console / Settings /
Properties / Mods / Notifications / Wizard) and every button on the
built AppImage — the agent cannot view the UI; `pnpm run check` and
`pnpm test` pass but that is not a visual verification.
Needed: after a fresh `packaging/build-linux.sh` you open
`dist/Minewrap-x86_64.AppImage`, run the wizard with the fake server,
start, read the console, stop, simulate a crash, edit settings,
and confirm no `[object Object]`.

## B-2: Wayland dialog check
What: the Tauri folder/file dialog under xdg-desktop-portal on your
Wayland stack (dialog:default capability) — the manual-entry
degradation also needs a pointer-free pass.
Needed: confirm the native folder dialog appears; if not, tell me
and I will keep the text field + validation.

## B-3: Cobblemon-server real test
What: the 6–8 GB NeoForge 1.21.1 server (`/home/kimhoanglong/Cobblemon-server`).
Excluded from agent tests per safety rule (§0.6).
Needed: the user runs `docs/FINAL_REPORT.md` steps; agent never
starts or modifies that directory.
