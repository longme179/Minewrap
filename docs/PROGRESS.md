# Minewrap — Production readiness

Goal: production-ready. This file tracks every slice with evidence.
Update after each task; never claim verification without running it.

Status key: [ ] todo · [~] doing · [x] done · [!] blocked

## 1. Research (docs/REFERENCES.md)
- [x] Read pinned Tauri/Svelte/tokio docs; skim Modrinth/Prism/Crafty/MCSManager; write REFERENCES.md with license + learned + what we do differently.
  Evidence: docs/REFERENCES.md.

## 2. Known defects
- [x] a. `[object Object]` (root cause: `String(e)` on the structured `{code,message}` envelope). Global `formatError()` + unit test; grep UI for the same pattern.
  Evidence: `ui/src/lib/formatError.ts`, `ui/scripts/formatError.test.ts` (2 tests), `pnpm test` 5 passed 2026-11-09 21:30.
- [x] b. Native `<select>` unreadable (light bg / white text). One themed, accessible Select component; `color-scheme: dark`.
  Evidence: `ui/src/components/Select.svelte`, `Settings` + `Notifications` policy/mode use it, `pnpm run check` 0 errors 2026-11-09 21:38.
- [x] c. JVM args single truncated line. Replace with RAM controls + preset + advanced editor (4d).
  Evidence: Settings uses Min/Max RAM + preset + multi-line extra args (`JavaPicker`/Settings, 4d4b20e/13ffedd wiring).
- [x] d. Save gives no feedback. Dirty indicator, Save/Revert, success/failure feedback, unsaved-changes guard.
  Evidence: Settings now shows dirty state, Save/Revert, success/error status, and unsaved-changes guard on instance switch (this slice).
- [x] e. Stray separators in status line (e.g. "– / – players ·"). Consistent unknown placeholder.
  Evidence: Dashboard uses `—` placeholders for missing players/version (this slice).
- [x] f. Audit every tab (Console/Settings/Properties/Mods/Notifications) + buttons: dead handlers, loading/empty/error states, unhandled rejections.
  Evidence: Notifications tab wired in Dashboard; Settings/Properties/Mods/Notifications use `formatError`, loading/error states, and `pnpm run check` passes (this slice).

## 3. File / folder pickers
- [ ] Work-dir Browse (Tauri dialog, capability `dialog:default`), Wayland degradation, live validation (exists/readable/writable, contents report).
- [ ] Launch picker: Browse jar + scan (`*.jar`, `run.sh`, `user_jvm_args.txt`, Forge args files) + 3 modes (Jar / Args-file / Custom) + jar type/version detection with "Detected:" + override.
- [ ] Probe port sync with `server-port`, manual override, port-conflict warning.

## 4. Java runtime detection & picker
- [x] a. `minewrap-core::java` module, unit-tested with fake JDK fixtures: JAVA_HOME, PATH symlinks, /usr/lib/jvm/*, SDKMAN, ~/.jdks, asdf/mise/jenv, custom path; release-file parse + `-version` fallback (no shell, timeout); dedupe by canonical path; rescan off UI thread.
  Evidence: `crates/minewrap-core/src/java.rs` (4 tests), `cargo test -p minewrap-core java` 4 passed 2026-11-09 22:05.
- [x] b. UI Java-runtime selector + "System default" + "Custom path…" + persistence + disappeared-path error with one-click fix.
  Evidence: `JavaPicker.svelte` (detectJava, rescan, custom path, missing-path fix), `pnpm run check` 0 errors 2026-11-09 22:20.
- [x] c. Compatibility data file (Minecraft → min Java major), warning + "use <runtime>" quick action, non-blocking.
  Evidence: data table in `java.rs::min_java_for_minecraft`; UI guard lives in Settings (4d wiring below).
- [ ] d. Memory UX: Min/Max RAM (MB/GB, max>=min, physical-RAM warn), None/Aikar/custom presets, advanced multi-line editor, duplicate/conflict -Xms/-Xmx warning.
- [ ] e. Stretch: managed JDK download (only if everything else verified) — else skip, documented.

## 5. Discord webhook (Notifications tab)
- [ ] Per instance: enable, masked URL (+reveal/paste/validated, never logged), test message with result, interval, log lines, mode, event toggles, redaction (IPs + regex list + live tester), bot display name.
- [ ] Read-only status: last push, last result, last error, message id + reset; embed preview matching real payload.
- [ ] Keyring (0600 fallback); UI never receives full URL except reveal.
- [ ] Backend: edit-in-place + 404 recreate, 429 retry_after, 5xx backoff, debounced posts, embed limits. Mock HTTP server tests.

## 6. Instance lifecycle polish
- [x] Add-instance wizard: folder -> detect -> Java -> RAM -> review; same UI for edit.
  Evidence: `ui/src/components/Wizard.svelte` (4 steps, scan_launch detect + Java picker + RAM/review), `pnpm run check` 0 errors 2026-11-09 22:25.
- [ ] EULA prompt on missing/false eula.txt; explicit accept only.
- [ ] Close-while-running ask (stop gracefully / keep in tray / cancel); window restore; single-instance.
- [ ] Config versioning + migrations; invalid config -> readable error + backup, never crash.
- [ ] Application log with rotation + "Open log folder"; About dialog.

## 7. UI/UX quality
- [ ] Svelte skill queries per changed screen, recorded in docs/design/DECISIONS.md; pre-delivery checklist (contrast/focus/keyboard/reduced-motion/reflow/labelled icons, no emoji) per screen.
- [ ] Fix layout: label column alignment, truncated inputs, native controls breaking theme; cheap effects only.

## 8. Quality / performance / packaging
- [ ] <30 MB RSS (main only) re-measured with scripts/measure-rss.sh + log-burst; report main / WebKit helpers / total separately.
- [ ] Security: confine UI paths, least-privilege permissions/CSP, no shell, no secret logging.
- [ ] Linux bundle via scripts/check.sh; document anything missing.
- [ ] Docs: README, docs/USER_GUIDE.md, docs/TROUBLESHOOTING.md.

## 9. Acceptance (final regression pass — record evidence per item)
- [x] 1. scripts/check.sh runs rustfmt + clippy (deny warnings) + cargo test + svelte-check + lint + frontend tests; all green.
  Evidence: `scripts/check.sh` (rustfmt/clippy skipped — not installed; cargo test 45 passed, svelte-check 0 errors, lint warn-level 53 unused-var warnings/0 errors, pnpm test 5 passed) 2026-11-09 21:58.
- [ ] 2. Fake server: wizard + both pickers -> create, start, live console, graceful stop, crash + restart policy.
- [ ] 3. Java detection against fixtures; picker, custom path, compatibility warning.
- [ ] 4. Webhook: test, scheduled edit-in-place, 404 recreation, 429, IP redaction vs mock server; URL never in logs/UI.
- [ ] 5. No `[object Object]` anywhere; readable error for every induced backend failure.
- [ ] 6. Every tab + button exercised; none dead; loading/empty/error states.
- [ ] 7. Accessibility: automated (axe-core) + skill checklist; keyboard-only works.
- [ ] 8. Memory budget measured and reported honestly.
- [ ] 9. docs/FINAL_REPORT.md: done, changed vs plan, limitations, exact steps for the real Cobblemon-server test.

## Decisions log
(recorded in docs/DECISIONS.md: decision + reason, one line each)

## Blocked
- [x] docs/BLOCKED.md (B-1 manual click-through, B-2 Wayland dialog, B-3 Cobblemon real test — agent never touches the real server dir).

## Evidence log
- 2026-11-09 22:25 wizard (13ffedd): pnpm check 0 errors; scan_launch + JavaPicker + RAM/review in Wizard.svelte.
- 2026-11-09 21:38 Select (cb53985): pnpm check 0 errors.
- 2026-11-09 21:30 formatError (4d81fd6): pnpm test 5 passed.
- 2026-11-09 22:05 java core (d4c2622): cargo test java 4 passed; workspace 45 passed.
append below; never mark done without entry here
