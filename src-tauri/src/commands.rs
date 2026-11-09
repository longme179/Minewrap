//! Tauri commands — the IPC surface between the Svelte webview and Rust.
//!
//! Every command carries `node_id`; `NodeService` routes to the embedded
//! core (local) or the remote client. There is no local/remote branch in
//! UI code, and no command ever returns a secret.

use tauri::State;

use minewrap_core::{InstanceSnapshot, LogLine};

use crate::node_service::{NodeService, SnapshotDto};
use crate::types::{
    Action, DiscordTestResponse, MinewrapError, ModDto, NodeSummary, PropertiesDoc,
};

/// Node discovery (v1 "gui node store" equivalent).
#[tauri::command]
pub async fn list_nodes(state: State<'_, NodeService>) -> Result<Vec<NodeSummary>, MinewrapError> {
    state.list_nodes()
}

/// Instances on a node (v1 `GET /instances`).
#[tauri::command]
pub async fn list_instances(
    state: State<'_, NodeService>,
    node_id: String,
) -> Result<Vec<minewrap_core::InstanceSummary>, MinewrapError> {
    state.list_instances(&node_id)
}

/// Full snapshot (v1 `GET /instances/{id}`).
#[tauri::command]
pub async fn get_snapshot(
    state: State<'_, NodeService>,
    node_id: String,
    instance_id: String,
) -> Result<SnapshotDto, MinewrapError> {
    state.snapshot(&node_id, &instance_id)
}

/// Lifecycle actions (v1 `POST …/start|stop|restart`). The action itself
/// is Phase 4/5; the IPC shape and error envelope are stable now.
#[tauri::command]
pub async fn instance_start(
    state: State<'_, NodeService>,
    node_id: String,
    instance_id: String,
) -> Result<Action, MinewrapError> {
    let _ = (state, node_id, instance_id);
    Err(MinewrapError::offline("lifecycle not wired yet (Phase 4)"))
}

#[tauri::command]
pub async fn instance_stop(
    state: State<'_, NodeService>,
    node_id: String,
    instance_id: String,
) -> Result<Action, MinewrapError> {
    let _ = (state, node_id, instance_id);
    Err(MinewrapError::offline("lifecycle not wired yet (Phase 4)"))
}

#[tauri::command]
pub async fn instance_restart(
    state: State<'_, NodeService>,
    node_id: String,
    instance_id: String,
) -> Result<Action, MinewrapError> {
    let _ = (state, node_id, instance_id);
    Err(MinewrapError::offline("lifecycle not wired yet (Phase 4)"))
}

#[tauri::command]
pub async fn send_command(
    state: State<'_, NodeService>,
    node_id: String,
    instance_id: String,
    command: String,
) -> Result<Action, MinewrapError> {
    let _ = (state, node_id, instance_id, command);
    Err(MinewrapError::offline("lifecycle not wired yet (Phase 4)"))
}

/// Log backfill (v1 `GET …/logs?limit&since_seq`).
#[tauri::command]
pub async fn get_logs(
    state: State<'_, NodeService>,
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
    state: State<'_, NodeService>,
    node_id: String,
    instance_id: String,
) -> Result<PropertiesDoc, MinewrapError> {
    let _ = (state, node_id, instance_id);
    Err(MinewrapError::offline("properties not wired yet (Phase 4)"))
}

#[tauri::command]
pub async fn write_properties(
    state: State<'_, NodeService>,
    node_id: String,
    instance_id: String,
    doc: PropertiesDoc,
) -> Result<Action, MinewrapError> {
    let _ = (state, node_id, instance_id, doc);
    Err(MinewrapError::offline("properties not wired yet (Phase 4)"))
}

/// Mods manager (new in v2).
#[tauri::command]
pub async fn list_mods(
    state: State<'_, NodeService>,
    node_id: String,
    instance_id: String,
) -> Result<Vec<ModDto>, MinewrapError> {
    let _ = (state, node_id, instance_id);
    Err(MinewrapError::offline("mods not wired yet (Phase 4)"))
}

#[tauri::command]
pub async fn toggle_mod(
    state: State<'_, NodeService>,
    node_id: String,
    instance_id: String,
    name: String,
) -> Result<Action, MinewrapError> {
    let _ = (state, node_id, instance_id, name);
    Err(MinewrapError::offline("mods not wired yet (Phase 4)"))
}

/// Discord test push (v1 `POST …/discord/test`).
#[tauri::command]
pub async fn discord_test(
    state: State<'_, NodeService>,
    node_id: String,
    instance_id: String,
) -> Result<DiscordTestResponse, MinewrapError> {
    let _ = (state, node_id, instance_id);
    Err(MinewrapError::offline("discord not wired yet (Phase 4)"))
}

/// `GET /health` proxy (v1 `GET /health`).
#[tauri::command]
pub async fn health(state: State<'_, NodeService>, node_id: String) -> Result<Action, MinewrapError> {
    let _ = (state, node_id);
    Ok(Action {
        ok: true,
        detail: Some(minewrap_core::API_VERSION.to_string()),
    })
}

pub fn full_snapshot(_s: &InstanceSnapshot) -> () {}
