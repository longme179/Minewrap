//! Tauri commands — the IPC surface between the Svelte webview and Rust.
//!
//! Every command carries `node_id`; `NodeService` routes it to the
//! local daemon or a remote node over `minewrap-remote`. The GUI holds
//! no server state itself (v1.10.0 daemon-first) and no command ever
//! returns a secret.

use std::sync::Arc;

use tauri::State;

use minewrap_remote::types::{
    Action, DiscordTestResponse, InstanceSummaryDto, JavaRuntimeDto, LogLineDto, MinewrapError,
    ModDto, NodeSummary, PropertiesDoc, SnapshotDto,
};

use crate::node_service::NodeService;

/// Node discovery (registry: local + nodes.json).
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
    state.add_remote_node(input.name, input.url, input.token)
}

#[tauri::command]
pub async fn remove_node(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
) -> Result<Action, MinewrapError> {
    let ok = state.remove_remote_node(&node_id)?;
    Ok(Action { ok, detail: Some("removed".into()) })
}

/// Health-probe / retry-reconnect one node (the UI's retry affordance
/// when a node shows offline).
#[tauri::command]
pub async fn retry_node(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
) -> Result<Action, MinewrapError> {
    // Local node: re-run the auto-spawn path (probe -> spawn -> wait),
    // then report the resulting state.
    if node_id == "local" {
        if let Err(e) = state.ensure_local_daemon() {
            state.set_node_state("local", minewrap_remote::types::NodeState::Offline);
            return Err(MinewrapError::offline(e));
        }
    }
    match state.probe_node(&node_id).await {
        Ok(st) => Ok(Action { ok: true, detail: Some(format!("{st:?}").to_lowercase()) }),
        Err(e) => Err(e),
    }
}

/// Instances on a node (v1 `GET /instances`).
#[tauri::command]
pub async fn list_instances(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
) -> Result<Vec<InstanceSummaryDto>, MinewrapError> {
    state.list_instances(&node_id).await
}

/// Full snapshot (v1 `GET /instances/{id}`).
#[tauri::command]
pub async fn get_snapshot(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
) -> Result<SnapshotDto, MinewrapError> {
    state.snapshot(&node_id, &instance_id).await
}

/// Lifecycle actions (v1 `POST …/start|stop|restart`).
#[tauri::command]
pub async fn instance_start(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
) -> Result<Action, MinewrapError> {
    state.start(&node_id, &instance_id).await
}

#[tauri::command]
pub async fn instance_stop(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
) -> Result<Action, MinewrapError> {
    state.stop(&node_id, &instance_id).await
}

#[tauri::command]
pub async fn instance_restart(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
) -> Result<Action, MinewrapError> {
    state.restart(&node_id, &instance_id).await
}

#[tauri::command]
pub async fn send_command(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
    command: String,
) -> Result<Action, MinewrapError> {
    state.send_command(&node_id, &instance_id, &command).await
}

/// Log backfill (v1 `GET …/logs?limit&since_seq`).
#[tauri::command]
pub async fn get_logs(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
    limit: Option<usize>,
    since_seq: Option<u64>,
) -> Result<Vec<LogLineDto>, MinewrapError> {
    state
        .get_logs(&node_id, &instance_id, limit.unwrap_or(200), since_seq.unwrap_or(0))
        .await
}

/// `server.properties` read.
#[tauri::command]
pub async fn read_properties(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
) -> Result<PropertiesDoc, MinewrapError> {
    state.read_properties(&node_id, &instance_id).await
}

#[tauri::command]
pub async fn write_properties(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
    doc: PropertiesDoc,
) -> Result<Action, MinewrapError> {
    state.write_properties(&node_id, &instance_id, &doc.text).await
}

/// Mods manager.
#[tauri::command]
pub async fn list_mods(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
) -> Result<Vec<ModDto>, MinewrapError> {
    state.list_mods(&node_id, &instance_id).await
}

#[tauri::command]
pub async fn toggle_mod(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
    name: String,
) -> Result<Action, MinewrapError> {
    let action = state.toggle_mod(&node_id, &instance_id, &name).await?;
    Ok(Action {
        ok: true,
        detail: Some(if action.ok { "enabled" } else { "disabled" }.to_string()),
    })
}

/// Discord test push (v1 `POST …/discord/test`).
#[tauri::command]
pub async fn discord_test(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
) -> Result<DiscordTestResponse, MinewrapError> {
    state.discord_test(&node_id, &instance_id).await
}

/// `GET /health` proxy (v1 `GET /health`).
#[tauri::command]
pub async fn health(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
) -> Result<Action, MinewrapError> {
    state.health(&node_id).await
}

/// Config read/write (v1 `GET/PUT …/config` — masked view, write-only
/// webhook). The frontend never sees the webhook URL.
#[tauri::command]
pub async fn get_config(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
) -> Result<serde_json::Value, MinewrapError> {
    state.get_config(&node_id, &instance_id).await
}

#[tauri::command]
pub async fn update_config(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
    patch: serde_json::Value,
) -> Result<Action, MinewrapError> {
    state.update_config(&node_id, &instance_id, patch).await
}

/// Instance add (local node; validates + persists on the daemon).
#[tauri::command]
pub async fn add_instance(
    state: State<'_, Arc<NodeService>>,
    config: serde_json::Value,
) -> Result<Action, MinewrapError> {
    state.add_instance(config).await
}

#[tauri::command]
pub async fn remove_instance(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    instance_id: String,
) -> Result<Action, MinewrapError> {
    state.remove_instance(&node_id, &instance_id).await
}

/// Java detection — served by the local daemon (the machine the
/// servers run on).
#[tauri::command]
pub async fn detect_java(state: State<'_, Arc<NodeService>>) -> Result<Vec<JavaRuntimeDto>, MinewrapError> {
    state.detect_java().await
}

/// Work-dir scan (launch candidates + contents report), confined on
/// the daemon side.
#[tauri::command]
pub async fn scan_launch(
    state: State<'_, Arc<NodeService>>,
    node_id: String,
    work_dir: String,
) -> Result<serde_json::Value, MinewrapError> {
    state.scan_launch(&node_id, &work_dir).await
}
