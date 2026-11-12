//! Tauri entrypoint (v1.10.0 P0 daemon-first): loads `minewrap.toml`
//! to learn the local daemon's host/port/data_dir, auto-spawns
//! `minewrapd` when `/health` refuses, then runs one WS `/events`
//! forwarder per node, mapping daemon messages to the IPC events the
//! webview has always consumed (`logs:batch`, `state:changed`,
//! `nodes:changed`).
//!
//! Exit policy (breaking change in v1.10.0): closing the window only
//! closes the connections — the daemon (and every server in it) keeps
//! running, Discord keeps pushing. "Stop everything" is now the
//! daemon's SIGTERM path (`systemctl --user stop minewrapd` or
//! `kill -TERM`), which stops all instances gracefully then exits.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

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
    // First run: write a default config so both halves agree on
    // host/port/data_dir. Settings mirror `minewrap_core::DaemonConfig`.
    let _ = std::fs::write(
        &stable,
        "[daemon]\nhost = \"127.0.0.1\"\nport = 8080\ndata_dir = \"./data\"\n",
    );
    stable
}

/// One forwarder per node (task B7): a `/events` WS connection that
/// maps daemon messages to the webview IPC events, reconnects with
/// backoff, and backfills the log gap (`since_seq`) after reconnects.
async fn forward_node(handle: tauri::AppHandle, svc: Arc<NodeService>, node_id: String) {
    use futures_util::StreamExt;
    use minewrap_remote::types::NodeState;
    let mut backoff = Duration::from_secs(1);
    // Last delivered seq per instance (for reconnect backfill).
    let mut last_seq: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    loop {
        let client = match svc.client_of(&node_id, false) {
            Ok(c) => c,
            Err(_) => {
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        match client.events_connect().await {
            Ok((stream, _hello)) => {
                svc.set_node_state(&node_id, NodeState::Connected);
                let _ = handle.emit(
                    "nodes:changed",
                    serde_json::json!({"node_id": node_id, "state": "connected"}),
                );
                backoff = Duration::from_secs(1);
                let mut read = stream;
                loop {
                    match read.next().await {
                        Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                            let v: serde_json::Value =
                                serde_json::from_str(&text).unwrap_or(serde_json::json!({}));
                            match v["type"].as_str() {
                                Some("state_change") => {
                                    let payload = serde_json::json!({
                                        "node_id": node_id,
                                        "instance_id": v["instance_id"],
                                        "old_state": v["old_state"],
                                        "new_state": v["new_state"],
                                        "reason": v["reason"],
                                    });
                                    let _ = handle.emit("state:changed", payload);
                                }
                                Some("log_batch") => {
                                    if let Some(last) =
                                        v["last_seq"].as_u64()
                                    {
                                        last_seq
                                            .insert(v["instance_id"].as_str().unwrap_or("").to_string(), last);
                                    }
                                    let payload = serde_json::json!({
                                        "node_id": node_id,
                                        "instance_id": v["instance_id"],
                                        "lines": v["lines"],
                                        "first_seq": v["first_seq"],
                                        "last_seq": v["last_seq"],
                                    });
                                    let _ = handle.emit("logs:batch", payload);
                                }
                                Some("instances_changed") => {
                                    let _ = handle.emit(
                                        "nodes:changed",
                                        serde_json::json!({"node_id": node_id}),
                                    );
                                }
                                _ => {}
                            }
                        }
                        Some(Ok(_)) => {}
                        Some(Err(_)) | None => break,
                    }
                }
                // Socket dropped: mark offline, reconnect below.
                svc.set_node_state(&node_id, NodeState::Offline);
                let _ = handle.emit(
                    "nodes:changed",
                    serde_json::json!({"node_id": node_id, "state": "offline"}),
                );
            }
            Err(e) => {
                let state = minewrap_remote::RemoteClient::state_from_error(&e);
                svc.set_node_state(&node_id, state);
            }
        }
        // Backfill the gap for instances we were streaming, then retry.
        if let Ok(client) = svc.client_of(&node_id, false) {
            for (instance_id, since) in last_seq.clone() {
                if let Ok(lines) = client.logs_typed(&instance_id, 200, since).await {
                    if !lines.is_empty() {
                        let first = lines.first().map(|l| l.seq).unwrap_or(0);
                        let last = lines.last().map(|l| l.seq).unwrap_or(0);
                        let payload = serde_json::json!({
                            "node_id": node_id,
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
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

fn main() {
    let config_path = default_config_path();
    let config_str = config_path.to_string_lossy().to_string();
    let node_service = match NodeService::load(&config_str) {
        Ok(svc) => Arc::new(svc),
        Err(e) => {
            eprintln!(
                "minewrap: failed to load config {config_str}: {e}; writing defaults and retrying"
            );
            let _ = std::fs::write(
                &config_path,
                "[daemon]\nhost = \"127.0.0.1\"\nport = 8080\ndata_dir = \"./data\"\n",
            );
            Arc::new(
                NodeService::load(&config_str)
                    .expect("config rewritten with defaults; second load must succeed"),
            )
        }
    };

    // Auto-spawn the local daemon (blocking, bounded ~11 s worst case).
    if let Err(e) = node_service.ensure_local_daemon() {
        eprintln!("minewrap: local daemon unavailable: {e}");
        node_service.set_node_state("local", minewrap_remote::types::NodeState::Offline);
    }

    let svc_for_setup = node_service.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(node_service.clone())
        .setup(move |app| {
            let handle = app.handle().clone();
            // One forwarder per node on the shared Tauri async runtime
            // (no dedicated thread+runtime anymore — gap G3).
            for node in svc_for_setup.list_nodes().unwrap_or_default() {
                let h = handle.clone();
                let svc = svc_for_setup.clone();
                let id = node.id;
                tauri::async_runtime::spawn(async move {
                    forward_node(h, svc, id).await;
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_nodes,
            commands::add_node,
            commands::remove_node,
            commands::retry_node,
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
        .run(|_app_handle, _event| {
            // v1.10.0 exit policy: closing the GUI only closes the
            // window (and the forwarder sockets die with the process).
            // Servers live in the daemon; stop them via
            // `systemctl --user stop minewrapd` or `kill -TERM`.
        });
}
