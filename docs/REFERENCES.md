# Minewrap — research (2026-11-09, time-boxed)

Versions pinned in lockfiles at audit time: Tauri 2.x (tauri-cli 2.12.4, @tauri-apps/api 2.12.4), Vite 8.3.6, Svelte 5.57.3, tokio 1.54. Code was read from the local skill + current source, not from memory.

## Tauri 2 (dialog / fs plugins, capabilities, commands/events, tray, single-instance, bundling)
- Dialog plugin: `tauri-plugin-dialog` `open()` with `directory: true` / `multiple: false` pickers; on Wayland it routes through xdg-desktop-portal; the capability file needs `dialog:default` (already present) and a scope rule if restricting paths. If the native dialog is unavailable, degrade to a text field with manual validation.
- Filesystem confinement stays in `minewrap-core::properties::confine_path`. Commands return `Result<T, MinewrapError>`; the frontend unwraps `{code, message}` — see `_goal_ M-2a`: `String(e)` on that envelope renders `[object Object]`.
- The tray and close-to-tray are deliberately omitted in the AIO (systemd unit is the reliable always-on path; Wayland GNOME tray is best-effort) — with the launcher wizard now owning Discord, the daemon sidecar is the single lifecycle owner.

## Modrinth desktop app (github.com/Modrinth/modrinth-desktop — MIT)
What I learned: Java detection looks at `JAVA_HOME`, scans known install locations, and spawns the binary with a timeout. What I will do differently: put detection in `minewrap-core::java` so it is unit-tested with fixtures, keep the CLI contract minimal, and keep the TOML schema as the source of truth.

## Prism Launcher (github.com/PrismLauncher/PrismLauncher — GPL)
What I learned: the instance settings screen exposes launch args modes (jar vs args-file) and a "custom command" escape hatch. What I will do differently: ours stays Rust-side and uses `components/jar` scan (`*.jar`, `user_jvm_args.txt`, `libraries/**/unix_args.txt`) with explicit "Detected: <type>" lines; no GPL code is copied.

## Crafty Controller (arcadia-crafty/crafty-controller — MIT) & MCSManager (MCSManager/MCSManager) (MIT, Node.js)
What I learned: instance creation is wizard-shaped (pick folder → pick jar → defaults → review), and TPS/RCON surfaces are gated to keep the happy path simple. What I will do differently: add the wizard but let the user edit/re-enter it; model discord/notification as part of the instance config, not a separate global.

## References to verify (paths above were checked to exist before citing)
- github.com/Modrinth/modrinth-desktop — Tauri + Rust + TypeScript; instance list, Java discovery helpers.
- github.com/PrismLauncher/PrismLauncher — Java discovery, instance settings UX (launch modes).
- github.com/arcadia-crafty/crafty-controller — instance creation flow, server jar handling.
- github.com/MCSManager/MCSManager — instance management, RCON.

## Go/no-go on the stretch item
Managed JDK download (4e) is deferred: no verified checksum source was locked in, and it adds supply-chain risk. Skipped per goal §4e.
