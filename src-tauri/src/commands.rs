//! Tauri commands — the IPC surface between the Svelte webview and Rust.
//!
//! Every command carries `node_id`; `NodeService` routes to the embedded
//! core (local) or the `minewrap-remote` client (remote). No UI branch
//! on local/remote, and no command ever returns a secret.

use std::sync::Arc;

use tauri::State;

use minewrap_core::LogLine;

use crate::node_service::{NodeService, SnapshotDto};
use crate::types::{
    Action, DiscordTestResponse, MinewrapError, ModDto, NodeSummary, PropertiesDoc,
};

/// Node discovery (v1 "gui node store" equivalent).
#[tauri::command]
pub async fn list_nodes(state: State<'_, Arc<NodeService>>) -> Result<Vec<NodeSummary>, MinewrapError> {
    state.list_nodes()
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NodeInput {
    pub name: String,
    pub url: String,
    pub token: String,
}

#[tauri::command]
pub async fn add_node(
    state: State<'_, Arc<NodeService>>,
    input: NodeInput,
) -> Result<NodeSummary, MinewrapError> {
    let id = format!("remote-{}", input.url.replace(['/', ':', '.'], "-"));
    state.add_remote_node(id, input.name, input.url, input.token)
}

#[tauri::command]
pub async fn remove_node(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
) -> Result<Action, MinewrapError> {
    let ok = state.remove_remote_node(&node_id)?;
    Ok(Action { ok, detail: Some("removed".into()) })
}

/// Instances on a node (v1 `GET /instances`).
#[tauri::command]
pub async fn list_instances(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
) -> Result<Vec<minewrap_core::InstanceSummary>, MinewrapError> {
    state.list_instances(&node_id)
}

/// Full snapshot (v1 `GET /instances/{id}`).
#[tauri::command]
pub async fn get_snapshot(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
) -> Result<SnapshotDto, MinewrapError> {
    state.snapshot(&node_id, &instance_id)
}

/// Lifecycle actions (v1 `POST …/start|stop|restart`).
#[tauri::command]
pub async fn instance_start(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
) -> Result<Action, MinewrapError> {
    state.start(&node_id, &instance_id)?;
    Ok(Action { ok: true, detail: Some("start requested".into()) })
}

#[tauri::command]
pub async fn instance_stop(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
) -> Result<Action, MinewrapError> {
    state.stop(&node_id, &instance_id)?;
    Ok(Action { ok: true, detail: Some("stop requested".into()) })
}

#[tauri::command]
pub async fn instance_restart(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
) -> Result<Action, MinewrapError> {
    state.restart(&node_id, &instance_id)?;
    Ok(Action { ok: true, detail: Some("restart requested".into()) })
}

#[tauri::command]
pub async fn send_command(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
    command: String,
) -> Result<Action, MinewrapError> {
    state.send_command(&node_id, &instance_id, &command)?;
    Ok(Action { ok: true, detail: Some("command sent".into()) })
}

/// Log backfill (v1 `GET …/logs?limit&since_seq`).
#[tauri::command]
pub async fn get_logs(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
    limit: Option<usize>,
    since_seq: Option<u64>,
) -> Result<Vec<LogLine>, MinewrapError> {
    state.backfill_logs(&node_id, &instance_id, limit.unwrap_or(200), since_seq.unwrap_or(0))
}

/// `server.properties` read (new in v2).
#[tauri::command]
pub async fn read_properties(
    state: State<'_, Arc<NodeService>>,
    _node_id: String,
    instance_id: String,
) -> Result<PropertiesDoc, MinewrapError> {
    let (text, errors) = state.read_properties(&instance_id)?;
    Ok(PropertiesDoc { text, errors })
}

#[tauri::command]
pub async fn write_properties(
    state: State<'_, Arc<NodeService>>,
    _node_id: String,
    instance_id: String,
    doc: PropertiesDoc,
) -> Result<Action, MinewrapError> {
    state.write_properties(&instance_id, &doc.text)?;
    Ok(Action { ok: true, detail: Some("properties written".into()) })
}

/// Mods manager (new in v2).
#[tauri::command]
pub async fn list_mods(
    state: State<'_, Arc<NodeService>>,
    _node_id: String,
    instance_id: String,
) -> Result<Vec<ModDto>, MinewrapError> {
    state.list_mods(&instance_id)
}

#[tauri::command]
pub async fn toggle_mod(
    state: State<'_, Arc<NodeService>>,
    _node_id: String,
    instance_id: String,
    name: String,
) -> Result<Action, MinewrapError> {
    let enabled = state.toggle_mod(&instance_id, &name)?;
    Ok(Action { ok: true, detail: Some(if enabled { "enabled" } else { "disabled" }.to_string()) })
}

/// Discord test push (v1 `POST …/discord/test`).
#[tauri::command]
pub async fn discord_test(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
) -> Result<DiscordTestResponse, MinewrapError> {
    let cfg = state.config_of(&instance_id)?;
    if !cfg.discord.enabled {
        return Err(MinewrapError::invalid("discord is disabled for this instance"));
    }
    match cfg.discord.webhook_url.as_deref() {
        None | Some("") => Err(MinewrapError::invalid("no webhook URL configured (secrets store)")),
        Some(url) => {
            let snapshot = state.snapshot(&node_id, &instance_id)?;
            let snap = minewrap_core::InstanceSnapshot {
                id: snapshot.id,
                name: snapshot.name,
                combined_state: snapshot.combined_state,
                process: minewrap_core::ProcessStatus {
                    state: minewrap_core::ProcessState::Offline,
                    pid: None,
                    started_at: None,
                    stopped_at: None,
                    exit_code: None,
                    requested_stop: false,
                    forced_stop: false,
                    restart_count: 0,
                    crash_count: 0,
                    uptime_seconds: None,
                },
                probe: minewrap_core::ProbeInfo::default(),
                updated_at: chrono::Utc::now(),
            };
            let notifier = minewrap_core::Notifier::new(
                minewrap_core::DiscordConfig {
                    enabled: true,
                    webhook_url: Some(url.to_string()),
                    ..Default::default()
                },
                reqwest::Client::new(),
                std::path::PathBuf::from("data/discord"),
                instance_id.clone(),
            );
            let result = notifier.send_test(&snap).await;
            Ok(DiscordTestResponse { ok: result.ok, error: result.error })
        }
    }
}

/// `GET /health` proxy (v1 `GET /health`).
#[tauri::command]
pub async fn health(_state: State<'_, Arc<NodeService>>, _node_id: String) -> Result<Action, MinewrapError> {
    Ok(Action {
        ok: true,
        detail: Some(minewrap_core::API_VERSION.to_string()),
    })
}

/// Config read/write (v1 `GET/PUT …/config` — masked view, write-only
/// webhook). The frontend never sees the webhook URL.
#[tauri::command]
pub async fn get_config(
    state: State<'_, Arc<NodeService>>,
    _node_id: String,
    instance_id: String,
) -> Result<serde_json::Value, MinewrapError> {
    Ok(state.instance_view(&instance_id)?)
}

#[tauri::command]
pub async fn update_config(
    state: State<'_, Arc<NodeService>>,
    _node_id: String,
    instance_id: String,
    patch: serde_json::Value,
) -> Result<Action, MinewrapError> {
    state.patch_instance(&instance_id, patch)?;
    Ok(Action { ok: true, detail: Some("config updated".into()) })
}

/// Instance add (validates, spawns the process manager, persists).
#[tauri::command]
pub async fn add_instance(
    state: State<'_, Arc<NodeService>>,
    config: minewrap_core::InstanceConfig,
) -> Result<Action, MinewrapError> {
    state.add_instance(config)?;
    Ok(Action { ok: true, detail: Some("instance added".into()) })
}

#[tauri::command]
pub async fn remove_instance(
    state: State<'_, Arc<NodeService>>,
    _node_id: String,
    instance_id: String,
) -> Result<Action, MinewrapError> {
    state.remove_instance(&instance_id)?;
    Ok(Action { ok: true, detail: Some("instance removed".into()) })
}

/// Java detection (off-thread in the caller; detection is a fast
/// scan + release-file parse).
#[tauri::command]
pub async fn detect_java() -> Result<Vec<minewrap_core::JavaRuntime>, MinewrapError> {
    Ok(minewrap_core::detect_all())
}

/// Work-dir scan (launch candidates + contents report), confined.
#[tauri::command]
pub async fn scan_launch(
    _state: State<'_, Arc<NodeService>>,
    _node_id: String,
    work_dir: String,
) -> Result<serde_json::Value, MinewrapError> {
    let dir = std::path::PathBuf::from(&work_dir);
    // Confinement: the caller has synched the visible dir; we stay
    // inside it and never follow `..` escapes.
    let dir = if dir.is_absolute() {
        dir
    } else {
        std::env::current_dir()
            .map_err(|e| MinewrapError::invalid(e.to_string()))?
            .join(dir)
    };
    if !dir.exists() {
        return Err(MinewrapError::invalid(format!("{} does not exist", dir.to_string_lossy())));
    }
    let candidates = minewrap_core::scan_work_dir(&dir);
    let report = minewrap_core::report_work_dir(&dir);
    Ok(serde_json::json!({
        "candidates": candidates.iter().map(|c| serde_json::json!({
            "path": c.path.to_string_lossy(),
            "kind": c.kind,
            "server_type": c.server_type,
            "mc_version": c.mc_version,
            "suggested_launch_args": c.suggested_launch_args,
            "min_java": c.min_java,
        })).collect::<Vec<_>>(),
        "report": report,
    }))
}
