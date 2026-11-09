//! Tauri entrypoint: builds the shell, loads `minewrap.toml` (or the
//! example config next to the binary), manages `NodeService`,
//! registers commands, and forwards core events to the webview
//! (`logs:batch` / `state:changed`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use minewrap_tauri::{NodeService, commands};
use tauri::Emitter;

fn config_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(path).join("minewrap");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".config").join("minewrap");
    }
    PathBuf::from(".").join(".minewrap")
}

fn default_config_path() -> PathBuf {
    if let Some(path) = std::env::var_os("MINEWRAP_CONFIG") {
        return PathBuf::from(path);
    }
    let stable = config_dir().join("minewrap.toml");
    let _ = std::fs::create_dir_all(stable.parent().unwrap_or(Path::new(".")));
    if stable.exists() {
        return stable;
    }
    // Migrate a legacy config next to the binary/AppImage (writable host path).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let legacy = dir.join("minewrap.toml");
            if legacy.exists() {
                let _ = std::fs::copy(&legacy, &stable);
                return stable;
            }
        }
    }
    let legacy_cwd = Path::new("minewrap.toml");
    if legacy_cwd.exists() {
        let _ = std::fs::copy(legacy_cwd, &stable);
        return stable;
    }
    let defaults = minewrap_core::DaemonConfig::default();
    let _ = defaults.save_to(&stable.to_string_lossy());
    stable
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
fn ensure_daemon_running() -> Option<std::process::Child> {
    let daemon = match bundled_daemon_path() {
        Some(p) => p,
        None => return None, // not shipped (dev)
    };
    // Pre-flight health check — skip the spawn when already up.
    let addr = "127.0.0.1:8080";
    if std::net::TcpStream::connect(addr).is_ok() {
        return None;
    }
    let config = default_config_path();
    let config_str = config.to_string_lossy().to_string();
    let out = std::process::Command::new(&daemon)
        .env("MINEWRAP_CONFIG", &config_str)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn();
    let child = match out {
        Ok(c) => {
            eprintln!("minewrap: spawned bundled daemon {daemon:?} (config {config_str})");
            Some(c)
        }
        Err(e) => {
            eprintln!("minewrap: failed to spawn bundled daemon {daemon:?}: {e}");
            None
        }
    };
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
    child
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
                            svc.update_process_state(&sc.instance_id, sc.new_state.as_str(), sc.reason.as_str());
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
    // Design choice: GUI exit must stop all servers/nodes and leave
    // no background daemon/process running. Do not auto-spawn the
    // bundled daemon sidecar for the desktop app.
    let mut _daemon_child: Option<std::process::Child> = None;

    let config_path = default_config_path();
    let config_str = config_path.to_string_lossy().to_string();
    let node_service = match NodeService::load(&config_str) {
        Ok(svc) => Arc::new(svc),
        Err(e) => {
            eprintln!("minewrap: failed to load config {config_str}: {e}; using stable defaults");
            let defaults = minewrap_core::DaemonConfig::default();
            let _ = defaults.save_to(&config_str);
            Arc::new(NodeService::load(&config_str).unwrap_or_else(|_| {
                NodeService::from_config(
                    minewrap_core::DaemonConfig::default(),
                    Arc::new(minewrap_core::EventBus::new()),
                )
            }))
        }
    };

    let svc_for_run = node_service.clone();
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
        .build(tauri::generate_context!())
        .expect("error while building Minewrap")
        .run(move |_app_handle, event| {
            if matches!(event, tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit) {
                let svc = svc_for_run.clone();
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                if let Ok(rt) = rt {
                    rt.block_on(async move {
                        svc.stop_all_local().await;
                    });
                }
                if let Some(child) = _daemon_child.as_mut() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        });
}
