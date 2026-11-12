//! Background runtime: per-node health polling, per-node instance lists,
//! one console WebSocket per selected instance with `since_seq` backfill,
//! and the command/control channel from the UI thread.
//!
//! Everything here runs on a current-thread Tokio runtime in its own OS
//! thread; the Slint event loop owns the UI thread. Messages cross via a
//! `std::sync::mpsc` channel drained by a Slint timer.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use minewrap_remote::types::{
    DiscordTestResponse, HealthResponse, JavaRuntimeDto, LogLineDto, ModDto, MinewrapError,
    NodeState, LOCAL_NODE_ID,
};
use crate::nodes::{NodeInstances, NodeService};
use serde_json::Value;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;

/// Commands sent from the UI thread to the runtime. Instance-scoped
/// commands carry `node_id` so every REST call goes to the right node.
pub enum Cmd {
    /// `start` / `stop` / `restart` route for `(node, instance)`.
    Control(String, String, String),
    /// Server stdin command (`POST …/command`).
    SendCommand(String, String, String),
    /// `POST /api/v1/instances` — create payload (local node only).
    AddInstance(Value),
    /// `DELETE /api/v1/instances/{id}`.
    RemoveInstance(String, String),
    /// `GET …/config`.
    GetConfig(String, String),
    /// `PUT …/config` patch (Settings panel).
    UpdateConfig(String, String, Value),
    /// `GET …/properties`.
    GetProperties(String, String),
    /// `PUT …/properties` body.
    WriteProperties(String, String, String),
    /// `GET …/mods`.
    ListMods(String, String),
    /// `POST …/mods/toggle` (fourth arg: the new enabled state to echo back).
    ToggleMod(String, String, String, bool),
    /// `PUT …/config` patch (Notifications panel, `discord` group only).
    NotifSave(String, String, Value),
    /// `POST …/discord/test`.
    DiscordTest(String, String),
    /// `GET /api/v1/java` (local node only).
    DetectJava,
    /// `POST /api/v1/scan-workdir`.
    ScanWorkdir(String, String),
    /// Immediate health probe of one node (Add/retry affordance).
    ProbeNode(String),
    /// Add a remote node to the registry (`nodes.json`) + probe it.
    AddNode {
        name: String,
        url: String,
        token: String,
    },
    /// Remove a remote node from the registry.
    RemoveNode(String),
}

/// Messages sent from the runtime to the UI thread.
pub enum UiMsg {
    /// All nodes: summary + state + their instance lists.
    Instances(Vec<NodeInstances>),
    /// One node's connection state changed.
    NodeState(String, NodeState),
    Health(Result<HealthResponse, String>),
    WsState(bool),
    Lines {
        instance: String,
        lines: Vec<LogLineDto>,
    },
    Notice(String),
    Error(String),
    /// Result of `add_instance`; the daemon answers `201 {"id","name"}` and
    /// `minewrap-remote` maps it explicitly (never decodes it as `Action`).
    InstanceAdded(Result<(), String>),
    /// Result of `remove_instance`, carries the removed id on success.
    InstanceRemoved(Result<String, String>),
    /// Result of `get_config` — raw `instance_view` JSON.
    ConfigFetched(Result<Value, String>),
    /// Result of `update_config` (Settings panel).
    ConfigSaved(Result<(), String>),
    /// Result of `GET …/properties`.
    PropertiesFetched(Result<Value, String>),
    /// Result of `PUT …/properties`.
    PropertiesSaved(Result<(), String>),
    /// Result of `GET …/mods`.
    ModsFetched(Result<Vec<ModDto>, String>),
    /// Result of `POST …/mods/toggle` (carries the new enabled state).
    ModsToggled(Result<bool, String>),
    /// Result of `PUT …/config` (Notifications panel).
    NotifSaved(Result<(), String>),
    /// Result of `POST …/discord/test`.
    DiscordTestFetched(Result<DiscordTestResponse, String>),
    /// Result of `detect_java`.
    JavaRuntimes(Result<Vec<JavaRuntimeDto>, String>),
    /// Result of `scan_workdir`.
    WorkDirScan(Result<Value, String>),
}

/// Spawn the runtime thread. Returns immediately; the thread lives until
/// the process exits.
pub fn spawn(
    rt: tokio::runtime::Runtime,
    svc: Arc<NodeService>,
    selected: watch::Receiver<Option<(String, String)>>,
    commands: mpsc::UnboundedReceiver<Cmd>,
    tx: std::sync::mpsc::Sender<UiMsg>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("minewrap-net".into())
        .spawn(move || {
            rt.block_on(async move {
                let poll = poll_loop(svc.clone(), tx.clone());
                let console = console_loop(svc.clone(), selected.clone(), tx.clone());
                let cmds = command_loop(svc, commands, tx);
                tokio::join!(poll, console, cmds);
            });
        })
        .expect("spawn network thread")
}

/// Every node: `/health` → state, then `GET /api/v1/instances`. An
/// immediate first pass runs before the first tick so the sidebar
/// populates without waiting 2 s.
async fn poll_loop(svc: Arc<NodeService>, tx: std::sync::mpsc::Sender<UiMsg>) {
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    poll_once(&svc, &tx).await;
    loop {
        tick.tick().await;
        poll_once(&svc, &tx).await;
    }
}

async fn poll_once(
    svc: &NodeService,
    tx: &std::sync::mpsc::Sender<UiMsg>,
) {
    let Ok(nodes) = svc.list_nodes() else {
        return;
    };
    let mut all = Vec::new();
    for node in &nodes {
        let state = match svc.probe_node(&node.id).await {
            Ok(s) => s,
            Err(e) => minewrap_remote::RemoteClient::state_from_error(&e),
        };
        let _ = tx.send(UiMsg::NodeState(node.id.clone(), state));
        let instances = match svc.list_instances(&node.id).await {
            Ok(l) => l,
            Err(_) => Vec::new(),
        };
        all.push(NodeInstances {
            node: node.clone(),
            instances,
        });
        // The status bar (daemon connected/offline + api version) reflects
        // the local node only.
        if node.id == LOCAL_NODE_ID {
            match svc.health(&node.id).await {
                Ok(h) => {
                    // `health()` returns an `Action`; map it to a
                    // `HealthResponse` for the status bar.
                    let _ = tx.send(UiMsg::Health(Ok(HealthResponse {
                        status: "ok".into(),
                        api_version: h.detail.unwrap_or_default(),
                        daemon_version: "".into(),
                    })));
                }
                Err(e) => {
                    let _ = tx.send(UiMsg::Health(Err(e.to_string())));
                }
            }
        }
    }
    let _ = tx.send(UiMsg::Instances(all));
}

/// Refresh the sidebar instance lists for every node (called after
/// instance mutations and after adding/removing nodes).
async fn refresh_instances(svc: &NodeService, tx: &std::sync::mpsc::Sender<UiMsg>) {
    match fetch_all_instances(svc).await {
        Ok(all) => {
            let _ = tx.send(UiMsg::Instances(all));
        }
        Err(e) => {
            let _ = tx.send(UiMsg::Error(format!("instances: {e}")));
        }
    }
}

async fn fetch_all_instances(
    svc: &NodeService,
) -> Result<Vec<NodeInstances>, MinewrapError> {
    let mut all = Vec::new();
    for node in svc.list_nodes()? {
        let instances = svc.list_instances(&node.id).await.unwrap_or_default();
        all.push(NodeInstances {
            node,
            instances,
        });
    }
    Ok(all)
}

/// One console socket for the selected instance; the watched selection
/// carries `(node_id, instance_id)`, so switching nodes reconnects the
/// socket to the right daemon. `since_seq` backfill covers disconnects.
async fn console_loop(
    svc: Arc<NodeService>,
    mut selected: watch::Receiver<Option<(String, String)>>,
    tx: std::sync::mpsc::Sender<UiMsg>,
) {
    let mut last_seq = 0u64;
    let mut current: Option<(String, String)> = selected.borrow().clone();
    loop {
        let Some((node_id, instance)) = current.clone() else {
            // Nothing selected: wait for the first pick.
            if selected.changed().await.is_err() {
                return;
            }
            current = selected.borrow().clone();
            last_seq = 0;
            continue;
        };

        let Ok(client) = svc.client_of(&node_id, false) else {
            let _ = tx.send(UiMsg::Error(format!("node {node_id}: no client")));
            if selected.changed().await.is_err() {
                return;
            }
            current = selected.borrow().clone();
            last_seq = 0;
            continue;
        };

        // REST `since_seq` backfill first (covers time spent disconnected);
        // the socket's initial `log` message replays the tail again and the
        // seq guard drops the overlap.
        match client.logs_typed(&instance, 200, last_seq).await {
            Ok(lines) => {
                let fresh: Vec<LogLineDto> = lines
                    .into_iter()
                    .filter(|l| l.seq > last_seq)
                    .collect();
                if let Some(max) = fresh.last().map(|l| l.seq) {
                    last_seq = last_seq.max(max);
                }
                if !fresh.is_empty() {
                    let _ = tx.send(UiMsg::Lines {
                        instance: instance.clone(),
                        lines: fresh,
                    });
                }
            }
            Err(e) => {
                let _ = tx.send(UiMsg::Error(format!("backfill: {e}")));
            }
        }

        let mut selection_changed = false;
        match client.console_connect(&instance).await {
            Ok(mut socket) => {
                let _ = tx.send(UiMsg::WsState(true));
                loop {
                    tokio::select! {
                        changed = selected.changed() => {
                            if changed.is_err() {
                                return;
                            }
                            current = selected.borrow().clone();
                            last_seq = 0;
                            selection_changed = true;
                            break;
                        }
                        msg = socket.next() => {
                            match msg {
                                Some(Ok(Message::Text(text))) => {
                                    let lines = parse_ws_text(&text);
                                    let fresh: Vec<LogLineDto> = lines
                                        .into_iter()
                                        .filter(|l| l.seq > last_seq)
                                        .collect();
                                    if let Some(max) = fresh.last().map(|l| l.seq) {
                                        last_seq = last_seq.max(max);
                                    }
                                    if !fresh.is_empty() {
                                        let _ = tx.send(UiMsg::Lines {
                                            instance: instance.clone(),
                                            lines: fresh,
                                        });
                                    }
                                }
                                Some(Ok(_)) => {}
                                Some(Err(e)) => {
                                    let _ = tx.send(UiMsg::Error(format!("ws: {e}")));
                                    break;
                                }
                                None => break,
                            }
                        }
                    }
                }
            }
            Err(e) => {
                let _ = tx.send(UiMsg::Error(format!("console ws: {e}")));
            }
        }
        let _ = tx.send(UiMsg::WsState(false));
        if selection_changed {
            continue;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// `/console/{id}` sends `{type:"log", lines:[…]}` (backfill) and
/// `{type:"log", line:{…}}` (live); other event types are ignored here.
fn parse_ws_text(text: &str) -> Vec<LogLineDto> {
    #[derive(serde::Deserialize)]
    struct Envelope {
        #[serde(rename = "type", default)]
        kind: String,
        #[serde(default)]
        lines: Vec<LogLineDto>,
        #[serde(default)]
        line: Option<LogLineDto>,
    }
    let Ok(msg) = serde_json::from_str::<Envelope>(text) else {
        return Vec::new();
    };
    if msg.kind != "log" {
        return Vec::new();
    }
    if let Some(line) = msg.line {
        return vec![line];
    }
    msg.lines
}

/// UI → REST: start/stop/restart, stdin commands and instance/node
/// lifecycle, all routed to the right node's client.
async fn command_loop(
    svc: Arc<NodeService>,
    mut commands: mpsc::UnboundedReceiver<Cmd>,
    tx: std::sync::mpsc::Sender<UiMsg>,
) {
    while let Some(cmd) = commands.recv().await {
        match cmd {
            Cmd::Control(ref node, ref instance, ref action) => {
                let client = match svc.client_of(node, false) {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx.send(UiMsg::Error(e.to_string()));
                        continue;
                    }
                };
                match client.action(instance, action, None).await {
                    Ok(v) => {
                        let ok = v
                            .get("ok")
                            .and_then(|x| x.as_bool())
                            .or_else(|| v.get("accepted").and_then(|x| x.as_bool()))
                            .unwrap_or(true);
                        let detail = v
                            .get("detail")
                            .and_then(|x| x.as_str())
                            .unwrap_or_default();
                        if ok {
                            let _ = tx.send(UiMsg::Notice(format!(
                                "{action}: accepted{}",
                                if detail.is_empty() { "" } else { " — " }
                            )));
                        } else {
                            let _ = tx.send(UiMsg::Error(format!("{action}: {detail}")));
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(UiMsg::Error(format!("{action}: {e}")));
                    }
                }
            }
            Cmd::SendCommand(ref node, ref instance, ref text) => {
                let client = match svc.client_of(node, false) {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx.send(UiMsg::Error(e.to_string()));
                        continue;
                    }
                };
                match client.send_command(instance, text).await {
                    Ok(v) => {
                        let accepted = v
                            .get("accepted")
                            .and_then(|x| x.as_bool())
                            .unwrap_or(true);
                        if accepted {
                            let _ = tx.send(UiMsg::Notice(format!("sent: {text}")));
                        } else {
                            let _ = tx.send(UiMsg::Error(format!("command rejected: {text}")));
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(UiMsg::Error(format!("command: {e}")));
                    }
                }
            }
            Cmd::AddInstance(ref config) => match svc.client_of(LOCAL_NODE_ID, false) {
                Ok(client) => match client.add_instance(config.clone()).await {
                    Ok(_) => {
                        let _ = tx.send(UiMsg::InstanceAdded(Ok(())));
                    }
                    Err(e) => {
                        let _ = tx.send(UiMsg::InstanceAdded(Err(format!("add instance: {e}"))));
                    }
                },
                Err(e) => {
                    let _ = tx.send(UiMsg::InstanceAdded(Err(format!("add instance: {e}"))));
                }
            },
            Cmd::RemoveInstance(ref node, ref id) => {
                let client = match svc.client_of(node, false) {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx.send(UiMsg::InstanceRemoved(Err(format!(
                            "remove instance: {e}"
                        ))));
                        continue;
                    }
                };
                match client.remove_instance(id).await {
                    Ok(_) => {
                        let _ = tx.send(UiMsg::InstanceRemoved(Ok(id.clone())));
                    }
                    Err(e) => {
                        let _ = tx.send(UiMsg::InstanceRemoved(Err(format!(
                            "remove instance: {e}"
                        ))));
                    }
                }
            }
            Cmd::GetConfig(ref node, ref id) => {
                let client = match svc.client_of(node, false) {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx.send(UiMsg::ConfigFetched(Err(format!("get config: {e}"))));
                        continue;
                    }
                };
                match client.get_config(id).await {
                    Ok(v) => {
                        let _ = tx.send(UiMsg::ConfigFetched(Ok(v)));
                    }
                    Err(e) => {
                        let _ = tx.send(UiMsg::ConfigFetched(Err(format!("get config: {e}"))));
                    }
                }
            }
            Cmd::UpdateConfig(ref node, ref id, ref patch) => {
                let client = match svc.client_of(node, false) {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx.send(UiMsg::ConfigSaved(Err(format!("update config: {e}"))));
                        continue;
                    }
                };
                match client.update_config(id, patch.clone()).await {
                    Ok(_) => {
                        let _ = tx.send(UiMsg::ConfigSaved(Ok(())));
                    }
                    Err(e) => {
                        let _ = tx.send(UiMsg::ConfigSaved(Err(format!(
                            "update config: {e}"
                        ))));
                    }
                }
            }
            Cmd::DetectJava => match svc.client_of(LOCAL_NODE_ID, false) {
                Ok(client) => match client.detect_java().await {
                    Ok(list) => {
                        let _ = tx.send(UiMsg::JavaRuntimes(Ok(list)));
                    }
                    Err(e) => {
                        let _ = tx.send(UiMsg::JavaRuntimes(Err(format!("detect java: {e}"))));
                    }
                },
                Err(e) => {
                    let _ = tx.send(UiMsg::JavaRuntimes(Err(format!("detect java: {e}"))));
                }
            },
            Cmd::ScanWorkdir(ref node, ref dir) => match svc.client_of(node, false) {
                Ok(client) => match client.scan_workdir(dir).await {
                    Ok(v) => {
                        let _ = tx.send(UiMsg::WorkDirScan(Ok(v)));
                    }
                    Err(e) => {
                        let _ = tx.send(UiMsg::WorkDirScan(Err(format!("scan workdir: {e}"))));
                    }
                },
                Err(e) => {
                    let _ = tx.send(UiMsg::WorkDirScan(Err(format!("scan workdir: {e}"))));
                }
            },
            Cmd::GetProperties(ref node, ref id) => match svc.client_of(node, false) {
                Ok(client) => match client.read_properties(id).await {
                    Ok(doc) => {
                        let v = serde_json::json!({ "text": doc.text, "errors": doc.errors });
                        let _ = tx.send(UiMsg::PropertiesFetched(Ok(v)));
                    }
                    Err(e) => {
                        let _ = tx.send(UiMsg::PropertiesFetched(Err(format!(
                            "read properties: {e}"
                        ))));
                    }
                },
                Err(e) => {
                    let _ = tx.send(UiMsg::PropertiesFetched(Err(format!(
                        "read properties: {e}"
                    ))));
                }
            },
            Cmd::WriteProperties(ref node, ref id, ref text) => match svc.client_of(node, false) {
                Ok(client) => match client.write_properties(id, text).await {
                    Ok(_) => {
                        let _ = tx.send(UiMsg::PropertiesSaved(Ok(())));
                    }
                    Err(e) => {
                        let _ = tx.send(UiMsg::PropertiesSaved(Err(format!(
                            "write properties: {e}"
                        ))));
                    }
                },
                Err(e) => {
                    let _ = tx.send(UiMsg::PropertiesSaved(Err(format!(
                        "write properties: {e}"
                    ))));
                }
            },
            Cmd::ListMods(ref node, ref id) => match svc.client_of(node, false) {
                Ok(client) => match client.list_mods(id).await {
                    Ok(list) => {
                        let _ = tx.send(UiMsg::ModsFetched(Ok(list)));
                    }
                    Err(e) => {
                        let _ = tx.send(UiMsg::ModsFetched(Err(format!("list mods: {e}"))));
                    }
                },
                Err(e) => {
                    let _ = tx.send(UiMsg::ModsFetched(Err(format!("list mods: {e}"))));
                }
            },
            Cmd::ToggleMod(ref node, ref id, ref name, enabled) => match svc.client_of(node, false) {
                Ok(client) => match client.toggle_mod(id, name).await {
                    Ok(_) => {
                        let _ = tx.send(UiMsg::ModsToggled(Ok(enabled)));
                    }
                    Err(e) => {
                        let _ = tx.send(UiMsg::ModsToggled(Err(format!("toggle mod: {e}"))));
                    }
                },
                Err(e) => {
                    let _ = tx.send(UiMsg::ModsToggled(Err(format!("toggle mod: {e}"))));
                }
            },
            Cmd::NotifSave(ref node, ref id, ref patch) => match svc.client_of(node, false) {
                Ok(client) => match client.update_config(id, patch.clone()).await {
                    Ok(_) => {
                        let _ = tx.send(UiMsg::NotifSaved(Ok(())));
                    }
                    Err(e) => {
                        let _ = tx.send(UiMsg::NotifSaved(Err(format!("update config: {e}"))));
                    }
                },
                Err(e) => {
                    let _ = tx.send(UiMsg::NotifSaved(Err(format!("update config: {e}"))));
                }
            },
            Cmd::DiscordTest(ref node, ref id) => match svc.client_of(node, false) {
                Ok(client) => match client.discord_test(id).await {
                    Ok(r) => {
                        let _ = tx.send(UiMsg::DiscordTestFetched(Ok(r)));
                    }
                    Err(e) => {
                        let _ = tx.send(UiMsg::DiscordTestFetched(Err(format!(
                            "discord test: {e}"
                        ))));
                    }
                },
                Err(e) => {
                    let _ = tx.send(UiMsg::DiscordTestFetched(Err(format!(
                        "discord test: {e}"
                    ))));
                }
            },
            Cmd::ProbeNode(ref node) => match svc.probe_node(node).await {
                Ok(st) => {
                    let _ = tx.send(UiMsg::NodeState(node.clone(), st));
                }
                Err(e) => {
                    let state = minewrap_remote::RemoteClient::state_from_error(&e);
                    let _ = tx.send(UiMsg::NodeState(node.clone(), state));
                }
            },
            Cmd::AddNode {
                ref name,
                ref url,
                ref token,
            } => match svc.add_remote_node(name.clone(), url.clone(), token.clone()) {
                Ok(summary) => {
                    let id = summary.id.clone();
                    match svc.probe_node(&id).await {
                        Ok(st) => {
                            let _ = tx.send(UiMsg::NodeState(id.clone(), st));
                        }
                        Err(e) => {
                            let state = minewrap_remote::RemoteClient::state_from_error(&e);
                            let _ = tx.send(UiMsg::NodeState(id.clone(), state));
                        }
                    }
                    refresh_instances(&svc, &tx).await;
                    let _ = tx.send(UiMsg::Notice(format!("node {} added", summary.name)));
                }
                Err(e) => {
                    let _ = tx.send(UiMsg::Error(format!("add node: {e}")));
                }
            },
            Cmd::RemoveNode(ref id) => match svc.remove_remote_node(id) {
                Ok(true) => {
                    refresh_instances(&svc, &tx).await;
                    let _ = tx.send(UiMsg::Notice("node removed".into()));
                }
                Ok(false) => {
                    let _ = tx.send(UiMsg::Error(format!("node {id} not found")));
                }
                Err(e) => {
                    let _ = tx.send(UiMsg::Error(format!("remove node: {e}")));
                }
            },
        }
        // Reflect the new state immediately instead of waiting for the tick.
        let refresh = matches!(
            &cmd,
            Cmd::AddInstance(_)
                | Cmd::RemoveInstance(_, _)
                | Cmd::Control(_, _, _)
                | Cmd::AddNode { .. }
                | Cmd::RemoveNode(_)
        );
        if refresh {
            refresh_instances(&svc, &tx).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_batched_and_single_log_messages() {
        let batch = r#"{"type":"log","instance_id":"s","lines":[
            {"seq":1,"ts":1794401798000,"line":"a"},
            {"seq":2,"ts":1794401798100,"line":"b"}]}"#;
        let single = r#"{"type":"log","instance_id":"s",
            "line":{"seq":3,"ts":1794401798200,"line":"c"}}"#;
        assert_eq!(parse_ws_text(batch).len(), 2);
        assert_eq!(parse_ws_text(single)[0].seq, 3);
    }

    #[test]
    fn parses_daemon_ws_json_with_millisecond_ts() {
        let text = r#"{"type":"log","instance_id":"s",
            "line":{"seq":2030,"ts":1794401798153,"line":"session tick"}}"#;
        let lines = parse_ws_text(text);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].seq, 2030);
    }

    #[test]
    fn ignores_non_log_and_garbage_messages() {
        assert!(parse_ws_text(r#"{"type":"snapshot","snapshot":{}}"#).is_empty());
        assert!(parse_ws_text("not json").is_empty());
    }
}
