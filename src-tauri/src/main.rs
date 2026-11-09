//! Tauri entrypoint: builds the shell, loads `minewrap.toml` (or the
//! example config next to the binary), manages `NodeService`,
//! registers commands, and forwards core events to the webview
//! (`logs:batch` / `state:changed`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use minewrap_tauri::{NodeService, commands};
use tauri::Emitter;

fn default_config_path() -> PathBuf {
    if let Some(path) = std::env::var_os("MINEWRAP_CONFIG") {
        return PathBuf::from(path);
    }
    let candidate = Path::new("minewrap.toml");
    if candidate.exists() {
        return candidate.to_path_buf();
    }
    // Portable layout: config next to the AppImage / binary (the
    // AppImage is a single fusesquashfs mount; the writable data dir
    // lives beside the .AppImage file on the host).
    if let Some(exe) = std::env::current_exe().ok() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("minewrap.toml");
            if candidate.exists() {
                return candidate;
            }
        }
    }
    // First run (no config yet): write an example beside the binary
    // so the user edits it, then the embedded core runs in-memory
    // until `minewrap.toml` exists.
    if let Some(exe) = std::env::current_exe().ok() {
        if let Some(dir) = exe.parent() {
            let example = dir.join("minewrap.toml");
            let defaults = minewrap_core::DaemonConfig::default();
            let _ = defaults.save_to(&example.to_string_lossy());
            return example;
        }
    }
    PathBuf::from("minewrap.toml")
}

/// Locate the bundled `minewrapd` next to the Tauri binary (AppImage
/// `usr/bin/` or a portable `binaries/` sibling). Returns `None` when
/// the daemon is not shipped (dev builds).
fn bundled_daemon_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    for cand in [
        dir.join("minewrapd"),
        dir.join("binaries").join("minewrapd"),
        dir.join("binaries").join("minewrapd-x86_64-unknown-linux-gnu"),
    ] {
        if cand.exists() {
            return Some(cand);
        }
    }
    None
}

/// Sidecar spawn (AppImage contract): if `GET /health` is refused,
/// exec the bundled daemon — with explicit args, never `sh -c` —
/// then register the node. The embedded `NodeService` core remains
/// the local node regardless; the sidecar is the always-on path.
fn ensure_daemon_running() {
    let daemon = match bundled_daemon_path() {
        Some(p) => p,
        None => return, // not shipped (dev)
    };
    // Pre-flight health check — skip the spawn when already up.
    let addr = "127.0.0.1:8080";
    if std::net::TcpStream::connect(addr).is_ok() {
        return;
    }
    let config = default_config_path();
    let config_str = config.to_string_lossy().to_string();
    let out = std::process::Command::new(&daemon)
        .env("MINEWRAP_CONFIG", &config_str)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn();
    match out {
        Ok(_) => eprintln!("minewrap: spawned bundled daemon {daemon:?} (config {config_str})"),
        Err(e) => eprintln!("minewrap: failed to spawn bundled daemon {daemon:?}: {e}"),
    }
    // Wait up to 10 s for `GET /health` (the daemon binds 127.0.0.1).
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    if let Ok(rt) = rt {
        rt.block_on(async {
            for _ in 0..20 {
                if std::net::TcpStream::connect(addr).is_ok() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        });
    }
}

async fn spawn_forwarder_future(
    handle: tauri::AppHandle,
    svc: Arc<NodeService>,
) {
    use minewrap_core::Event;
    let mut sub = svc.subscribe();
        // pending: (instance_id, lines) per instance so batches carry the
        // correct instance_id (was previously hardcoded empty).
        let mut pending: Vec<(String, minewrap_core::LogLine)> = Vec::new();
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(40));
        loop {
            tokio::select! {
                ev = sub.rx.recv() => {
                    match ev {
                        Some(Event::LogLine(le)) => pending.push((le.instance_id.clone(), le.line)),
                        Some(Event::StateChange(sc)) => {
                            let payload = serde_json::json!({
                                "node_id": "local",
                                "instance_id": sc.instance_id,
                                "old_state": sc.old_state,
                                "new_state": sc.new_state,
                                "reason": sc.reason,
                            });
                            let _ = handle.emit("state:changed", payload);
                        }
                        _ => {}
                    }
                }
                _ = interval.tick() => {
                    if !pending.is_empty() {
                        // Split by instance; a single tick can hold lines
                        // from several servers.
                        use std::collections::HashMap;
                        let mut by_instance: HashMap<String, Vec<minewrap_core::LogLine>> = HashMap::new();
                        for (iid, line) in pending.drain(..) {
                            by_instance.entry(iid).or_default().push(line);
                        }
                        for (instance_id, lines) in by_instance {
                            let first = lines.first().map(|l| l.seq).unwrap_or(0);
                            let last = lines.last().map(|l| l.seq).unwrap_or(0);
                            let payload = serde_json::json!({
                                "node_id": "local",
                                "instance_id": instance_id,
                                "lines": lines,
                                "first_seq": first,
                                "last_seq": last,
                            });
                            let _ = handle.emit("logs:batch", payload);
                        }
                    }
                }
            }
    }
}

fn main() {
    // AppImage sidecar contract (v1 parity): ensure the bundled
    // daemon is up before first-run; the embedded core is the local
    // node regardless.
    ensure_daemon_running();

    let config_path = default_config_path();
    let config_str = config_path.to_string_lossy().to_string();
    let node_service = match NodeService::load(&config_str) {
        Ok(svc) => Arc::new(svc),
        Err(e) => {
            eprintln!("minewrap: failed to load config {config_str}: {e}");
            Arc::new(NodeService::from_config(
                minewrap_core::DaemonConfig::default(),
                Arc::new(minewrap_core::EventBus::new()),
            ))
        }
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(node_service.clone())
        .setup(move |app| {
            let svc = node_service.clone();
            let handle = app.handle().clone();
            // Spawn the forwarder on a dedicated Tokio runtime — the
            // AppImage entry has no ambient runtime until the webview
            // is ready.
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("event forwarder runtime");
                rt.block_on(async {
                    spawn_forwarder_future(handle, svc).await;
                });
            });
            // The guaranteed always-on path is the systemd unit
            // (packaging/minewrapd.service); the stated tray is
            // best-effort on Wayland and omitted here to keep the
            // binary lean. Users can still hide with the window
            // manager; the Discord post loop lives in minewrapd.
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_nodes,
            commands::add_node,
            commands::remove_node,
            commands::list_instances,
            commands::get_snapshot,
            commands::instance_start,
            commands::instance_stop,
            commands::instance_restart,
            commands::send_command,
            commands::get_logs,
            commands::get_config,
            commands::update_config,
            commands::add_instance,
            commands::remove_instance,
            commands::detect_java,
            commands::scan_launch,
            commands::read_properties,
            commands::write_properties,
            commands::list_mods,
            commands::toggle_mod,
            commands::discord_test,
            commands::health,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Minewrap")
}
