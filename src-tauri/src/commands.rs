//! Tauri commands — the IPC surface between the Svelte webview and Rust.
//!
//! Every command carries `node_id`; `NodeService` routes to the embedded
//! core (local) or the remote client. There is no local/remote branch in
//! UI code, and no command ever returns a secret.

use std::sync::Arc;

use tauri::State;

use minewrap_core::LogLine;

use crate::node_service::{NodeService, SnapshotDto};
use crate::types::{
    Action, DiscordTestResponse, MinewrapError, ModDto, NodeSummary, PropertiesDoc,
};

fn local_or_remote(svc: &NodeService, node_id: &str) -> Result<(), MinewrapError> {
    if svc.is_local(node_id) {
        Ok(())
    } else {
        Err(svc.remote_offline(node_id))
    }
}

/// Node discovery (v1 "gui node store" equivalent).
#[tauri::command]
pub async fn list_nodes(state: State<'_, Arc<NodeService>>) -> Result<Vec<NodeSummary>, MinewrapError> {
    state.list_nodes()
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
    local_or_remote(&state, &node_id)?;
    state.start(&node_id, &instance_id)?;
    Ok(Action { ok: true, detail: Some("start requested".into()) })
}

#[tauri::command]
pub async fn instance_stop(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
) -> Result<Action, MinewrapError> {
    local_or_remote(&state, &node_id)?;
    state.stop(&node_id, &instance_id)?;
    Ok(Action { ok: true, detail: Some("stop requested".into()) })
}

#[tauri::command]
pub async fn instance_restart(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
) -> Result<Action, MinewrapError> {
    local_or_remote(&state, &node_id)?;
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
    local_or_remote(&state, &node_id)?;
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
    node_id: String,
    instance_id: String,
) -> Result<PropertiesDoc, MinewrapError> {
    local_or_remote(&state, &node_id)?;
    let (text, errors) = state.read_properties(&instance_id)?;
    Ok(PropertiesDoc { text, errors })
}

#[tauri::command]
pub async fn write_properties(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
    doc: PropertiesDoc,
) -> Result<Action, MinewrapError> {
    local_or_remote(&state, &node_id)?;
    state.write_properties(&instance_id, &doc.text)?;
    Ok(Action { ok: true, detail: Some("properties written".into()) })
}

/// Mods manager (new in v2).
#[tauri::command]
pub async fn list_mods(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
) -> Result<Vec<ModDto>, MinewrapError> {
    local_or_remote(&state, &node_id)?;
    state.list_mods(&instance_id)
}

#[tauri::command]
pub async fn toggle_mod(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
    name: String,
) -> Result<Action, MinewrapError> {
    local_or_remote(&state, &node_id)?;
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
    local_or_remote(&state, &node_id)?;
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
pub async fn health(state: State<'_, Arc<NodeService>>, node_id: String) -> Result<Action, MinewrapError> {
    local_or_remote(&state, &node_id)?;
    Ok(Action {
        ok: true,
        detail: Some(minewrap_core::API_VERSION.to_string()),
    })
}
