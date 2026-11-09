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
    if let Some(exe) = std::env::current_exe().ok() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("minewrap.toml");
            if candidate.exists() {
                return candidate;
            }
        }
    }
    PathBuf::from("minewrap.toml")
}

fn spawn_forwarder(
    handle: tauri::AppHandle,
    svc: Arc<NodeService>,
) {
    tokio::spawn(async move {
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
    });
}

fn main() {
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
            spawn_forwarder(handle, svc);
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
