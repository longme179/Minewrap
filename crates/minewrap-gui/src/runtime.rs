//! Background runtime: REST polling (instances + health), one console
//! WebSocket per selected instance with `since_seq` backfill, and the
//! command/control channel from the UI thread.
//!
//! Everything here runs on a current-thread Tokio runtime in its own OS
//! thread; the Slint event loop owns the UI thread. Messages cross via a
//! `std::sync::mpsc` channel drained by a Slint timer.

use std::time::Duration;

use futures_util::StreamExt;
use minewrap_remote::RemoteClient;
use minewrap_remote::types::{
    DiscordTestResponse, HealthResponse, InstanceSummaryDto, JavaRuntimeDto, LogLineDto, ModDto,
};
use serde_json::Value;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;

/// Commands sent from the UI thread to the runtime.
pub enum Cmd {
    /// `start` / `stop` / `restart` route for the selected instance.
    Control(String),
    /// Server stdin command (`POST …/command`).
    SendCommand(String),
    /// `POST /api/v1/instances` — create payload.
    AddInstance(Value),
    /// `DELETE /api/v1/instances/{id}`.
    RemoveInstance(String),
    /// `GET …/config`.
    GetConfig(String),
    /// `PUT …/config` patch (Settings panel).
    UpdateConfig(String, Value),
    /// `GET …/properties`.
    GetProperties(String),
    /// `PUT …/properties` body.
    WriteProperties(String, String),
    /// `GET …/mods`.
    ListMods(String),
    /// `POST …/mods/toggle` (third arg: the new enabled state to echo back).
    ToggleMod(String, String, bool),
    /// `PUT …/config` patch (Notifications panel, `discord` group only).
    NotifSave(String, Value),
    /// `POST …/discord/test`.
    DiscordTest(String),
    /// `GET /api/v1/java`.
    DetectJava,
    /// `POST /api/v1/scan-workdir`.
    ScanWorkdir(String),
}

/// Messages sent from the runtime to the UI thread.
pub enum UiMsg {
    Instances(Vec<InstanceSummaryDto>),
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
    client: RemoteClient,
    selected: watch::Receiver<Option<String>>,
    commands: mpsc::UnboundedReceiver<Cmd>,
    tx: std::sync::mpsc::Sender<UiMsg>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("minewrap-net".into())
        .spawn(move || {
            rt.block_on(async move {
                let poll = poll_loop(client.clone(), tx.clone());
                let console = console_loop(client.clone(), selected.clone(), tx.clone());
                let cmds = command_loop(client.clone(), selected, commands, tx);
                tokio::join!(poll, console, cmds);
            });
        })
        .expect("spawn network thread")
}

/// Poll `/health` and `GET /api/v1/instances` every 2 s.
async fn poll_loop(client: RemoteClient, tx: std::sync::mpsc::Sender<UiMsg>) {
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        match client.health_timeout(Duration::from_secs(2)).await {
            Ok(h) => {
                let _ = tx.send(UiMsg::Health(Ok(h)));
            }
            Err(e) => {
                let _ = tx.send(UiMsg::Health(Err(e.to_string())));
            }
        }
        match client.list_instances_typed().await {
            Ok(list) => {
                let _ = tx.send(UiMsg::Instances(list));
            }
            Err(e) => {
                let _ = tx.send(UiMsg::Error(format!("instances: {e}")));
            }
        }
    }
}

/// One console socket for the selected instance; reconnects with backfill.
async fn console_loop(
    client: RemoteClient,
    mut selected: watch::Receiver<Option<String>>,
    tx: std::sync::mpsc::Sender<UiMsg>,
) {
    let mut last_seq = 0u64;
    let mut current = selected.borrow().clone();
    loop {
        let Some(instance) = current.clone() else {
            // Nothing selected: wait for the first pick.
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

/// UI → REST: start/stop/restart and stdin commands for the selection.
async fn command_loop(
    client: RemoteClient,
    selected: watch::Receiver<Option<String>>,
    mut commands: mpsc::UnboundedReceiver<Cmd>,
    tx: std::sync::mpsc::Sender<UiMsg>,
) {
    while let Some(cmd) = commands.recv().await {
        let instance = selected.borrow().clone();
        let Some(instance) = instance else {
            let _ = tx.send(UiMsg::Error("no instance selected".into()));
            continue;
        };
        match cmd {
            Cmd::Control(ref action) => {
                match client.action(&instance, action, None).await {
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
                            let _ = tx.send(UiMsg::Error(format!("{action}: {}", detail)));
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(UiMsg::Error(format!("{action}: {e}")));
                    }
                }
            }
            Cmd::SendCommand(ref text) => match client.send_command(&instance, text).await {
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
            },
            Cmd::AddInstance(ref config) => match client.add_instance(config.clone()).await {
                Ok(_) => {
                    let _ = tx.send(UiMsg::InstanceAdded(Ok(())));
                }
                Err(e) => {
                    let _ = tx.send(UiMsg::InstanceAdded(Err(format!("add instance: {e}"))));
                }
            },
            Cmd::RemoveInstance(ref id) => match client.remove_instance(id).await {
                Ok(_) => {
                    let _ = tx.send(UiMsg::InstanceRemoved(Ok(id.clone())));
                }
                Err(e) => {
                    let _ = tx.send(UiMsg::InstanceRemoved(Err(format!(
                        "remove instance: {e}"
                    ))));
                }
            },
            Cmd::GetConfig(ref id) => match client.get_config(id).await {
                Ok(v) => {
                    let _ = tx.send(UiMsg::ConfigFetched(Ok(v)));
                }
                Err(e) => {
                    let _ = tx.send(UiMsg::ConfigFetched(Err(format!("get config: {e}"))));
                }
            },
            Cmd::UpdateConfig(ref id, ref patch) => {
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
            Cmd::DetectJava => match client.detect_java().await {
                Ok(list) => {
                    let _ = tx.send(UiMsg::JavaRuntimes(Ok(list)));
                }
                Err(e) => {
                    let _ = tx.send(UiMsg::JavaRuntimes(Err(format!("detect java: {e}"))));
                }
            },
            Cmd::ScanWorkdir(ref dir) => match client.scan_workdir(dir).await {
                Ok(v) => {
                    let _ = tx.send(UiMsg::WorkDirScan(Ok(v)));
                }
                Err(e) => {
                    let _ = tx.send(UiMsg::WorkDirScan(Err(format!("scan workdir: {e}"))));
                }
            },
            Cmd::GetProperties(ref id) => match client.read_properties(id).await {
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
            Cmd::WriteProperties(ref id, ref text) => match client.write_properties(id, text).await
            {
                Ok(_) => {
                    let _ = tx.send(UiMsg::PropertiesSaved(Ok(())));
                }
                Err(e) => {
                    let _ = tx.send(UiMsg::PropertiesSaved(Err(format!(
                        "write properties: {e}"
                    ))));
                }
            },
            Cmd::ListMods(ref id) => match client.list_mods(id).await {
                Ok(list) => {
                    let _ = tx.send(UiMsg::ModsFetched(Ok(list)));
                }
                Err(e) => {
                    let _ = tx.send(UiMsg::ModsFetched(Err(format!("list mods: {e}"))));
                }
            },
            Cmd::ToggleMod(ref id, ref name, enabled) => match client.toggle_mod(id, name).await {
                Ok(_) => {
                    let _ = tx.send(UiMsg::ModsToggled(Ok(enabled)));
                }
                Err(e) => {
                    let _ = tx.send(UiMsg::ModsToggled(Err(format!("toggle mod: {e}"))));
                }
            },
            Cmd::NotifSave(ref id, ref patch) => match client.update_config(id, patch.clone()).await
            {
                Ok(_) => {
                    let _ = tx.send(UiMsg::NotifSaved(Ok(())));
                }
                Err(e) => {
                    let _ = tx.send(UiMsg::NotifSaved(Err(format!("update config: {e}"))));
                }
            },
            Cmd::DiscordTest(ref id) => match client.discord_test(id).await {
                Ok(r) => {
                    let _ = tx.send(UiMsg::DiscordTestFetched(Ok(r)));
                }
                Err(e) => {
                    let _ = tx.send(UiMsg::DiscordTestFetched(Err(format!(
                        "discord test: {e}"
                    ))));
                }
            },
        }
        // Reflect the new state immediately instead of waiting for the tick.
        let refresh = matches!(
            &cmd,
            Cmd::AddInstance(_) | Cmd::RemoveInstance(_) | Cmd::Control(_)
        );
        if refresh {
            if let Ok(list) = client.list_instances_typed().await {
                let _ = tx.send(UiMsg::Instances(list));
            }
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
