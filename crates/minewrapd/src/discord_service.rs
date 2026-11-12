//! Discord service (task B3, gap G1 + D5): the daemon pushes Discord
//! notifications itself, so they survive the GUI being closed.
//!
//! Two push paths (v1 `discord.py` semantics, now wired to the config
//! fields that were dead in v1.9.3):
//! 1. **State-change push** — on every `StateChange`, with a
//!    leading-edge + trailing debounce (`state_change_debounce_seconds`):
//!    the first change after a quiet period pushes immediately; changes
//!    inside the window coalesce into one push of the *latest* state.
//! 2. **Periodic push** — while an instance is online, the first push
//!    lands `first_push_delay_seconds` after becoming online, then
//!    every `interval_seconds`; `edit-in-place` PATCHes the persisted
//!    message id, `new-message` POSTs.
//!
//! D5: one shared `reqwest::Client` for the whole daemon (also used by
//! the `/discord/test` route) instead of a client per push.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use minewrap_core::events::{Event, WILDCARD};
use minewrap_core::{Notifier, build_state_change_embed};

use crate::instances::InstanceManager;

static HTTP: OnceLock<reqwest::Client> = OnceLock::new();

/// The daemon-wide shared HTTP client (D5).
pub fn shared_http() -> &'static reqwest::Client {
    HTTP.get_or_init(reqwest::Client::new)
}

#[derive(Default)]
struct PerInstance {
    /// Last time a state-change push actually went out.
    last_state_push: Option<Instant>,
    /// Coalesced state change waiting for the debounce window to close.
    pending: Option<minewrap_core::StateChange>,
    due_at: Option<Instant>,
    /// Next periodic push (`None` while not online).
    next_periodic: Option<Instant>,
}

/// Spawn the Discord service. Runs for the lifetime of the daemon.
pub fn spawn(mgr: Arc<InstanceManager>) -> tokio::task::JoinHandle<()> {
    let mut sub = mgr.subscribe(WILDCARD);
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tokio::spawn(async move {
        let mut per: HashMap<String, PerInstance> = HashMap::new();
        loop {
            tokio::select! {
                ev = sub.rx.recv() => {
                    match ev {
                        Some(Event::StateChange(sc)) => {
                            handle_state_change(&mgr, &mut per, sc);
                        }
                        Some(_) => {}
                        None => break,
                    }
                }
                _ = tick.tick() => {
                    fire_due(&mgr, &mut per);
                }
            }
        }
    })
}

fn discord_cfg_of(mgr: &InstanceManager, id: &str) -> Option<minewrap_core::DiscordConfig> {
    mgr.config_of(id).map(|c| c.discord)
}

// (config_of takes &self, so a plain reference is fine here)

fn enabled_and_hooked(cfg: &minewrap_core::DiscordConfig) -> bool {
    cfg.enabled && cfg.webhook_set()
}

fn handle_state_change(
    mgr: &Arc<InstanceManager>,
    per: &mut HashMap<String, PerInstance>,
    sc: minewrap_core::StateChange,
) {
    let Some(cfg) = discord_cfg_of(mgr, &sc.instance_id) else {
        return;
    };
    if !enabled_and_hooked(&cfg) {
        return;
    }
    let entry = per.entry(sc.instance_id.clone()).or_default();
    let now = Instant::now();
    let debounce = Duration::from_secs_f64(cfg.state_change_debounce_seconds.max(0.0));

    // Schedule/clear the periodic path around the online edge.
    if sc.new_state == "online" {
        entry.next_periodic =
            Some(now + Duration::from_secs_f64(cfg.first_push_delay_seconds.max(0.0)));
    } else if matches!(sc.new_state.as_str(), "offline" | "crashed" | "stopping") {
        entry.next_periodic = None;
    }

    match entry.last_state_push {
        Some(last) if now < last + debounce => {
            // Inside the window: coalesce, push the latest state when it
            // closes (trailing edge).
            entry.pending = Some(sc);
            entry.due_at = Some(last + debounce);
        }
        _ => {
            // Leading edge: push now.
            entry.last_state_push = Some(now);
            entry.pending = None;
            entry.due_at = None;
            spawn_push(mgr.clone(), sc, true);
        }
    }
}

/// Fire debounced (trailing) pushes and periodic pushes that are due.
/// The 1 s tick bounds the scheduling granularity.
fn fire_due(mgr: &Arc<InstanceManager>, per: &mut HashMap<String, PerInstance>) {
    let now = Instant::now();
    let mut due: Vec<minewrap_core::StateChange> = Vec::new();
    for (id, entry) in per.iter_mut() {
        if let (Some(p), Some(due_at)) = (entry.pending.take(), entry.due_at.take()) {
            if now >= due_at {
                entry.last_state_push = Some(now);
                due.push(p);
            } else {
                // Not yet: put it back.
                entry.pending = Some(p);
                entry.due_at = Some(due_at);
            }
        }
        // Periodic: only while the merged state is online.
        let online = mgr
            .snapshot_of(id)
            .map(|s| s.combined_state == "online")
            .unwrap_or(false);
        if !online {
            entry.next_periodic = None;
            continue;
        }
        if let Some(np) = entry.next_periodic {
            if now >= np
                && let Some(cfg) = discord_cfg_of(mgr, id)
                && enabled_and_hooked(&cfg)
            {
                spawn_periodic(mgr.clone(), id.clone(), cfg);
            }
            if now >= np {
                let interval = discord_cfg_of(mgr, id)
                    .map(|c| c.interval_seconds)
                    .unwrap_or(600.0);
                entry.next_periodic = Some(now + Duration::from_secs_f64(interval.max(1.0)));
            }
        } else if let Some(cfg) = discord_cfg_of(mgr, id) {
            // Daemon restarted while the server was already online:
            // (re)arm the first push.
            entry.next_periodic =
                Some(now + Duration::from_secs_f64(cfg.first_push_delay_seconds.max(0.0)));
        }
    }
    for sc in due {
        spawn_push(mgr.clone(), sc, true);
    }
}

/// One state-change notification (embed + status message), ported from
/// the v1.9.3 GUI forwarder.
fn spawn_push(mgr: Arc<InstanceManager>, sc: minewrap_core::StateChange, with_status: bool) {
    tokio::spawn(async move {
        let Some(cfg) = mgr.config_of(&sc.instance_id) else {
            return;
        };
        if !enabled_and_hooked(&cfg.discord) {
            return;
        }
        let Some(snap) = mgr.snapshot_of(&sc.instance_id) else {
            return;
        };
        let lines = mgr.backfill(&sc.instance_id, cfg.discord.log_lines, 0);
        let notifier = Notifier::new(
            cfg.discord.clone(),
            shared_http().clone(),
            mgr.data_dir().join("discord"),
            sc.instance_id.clone(),
        );
        if cfg.discord.state_change_posts {
            let embed = build_state_change_embed(&snap, &sc.old_state, &sc.new_state, &sc.reason);
            let _ = notifier
                .request(&format!("{}?wait=true", notifier.webhook()), "POST", &embed)
                .await;
        }
        if with_status {
            let _ = notifier.push_status(&snap, &lines).await;
        }
    });
}

/// Periodic status push (edit-in-place PATCH / new-message POST).
fn spawn_periodic(mgr: Arc<InstanceManager>, id: String, cfg: minewrap_core::DiscordConfig) {
    tokio::spawn(async move {
        let Some(snap) = mgr.snapshot_of(&id) else {
            return;
        };
        let lines = mgr.backfill(&id, cfg.log_lines, 0);
        let notifier = Notifier::new(
            cfg,
            shared_http().clone(),
            mgr.data_dir().join("discord"),
            id,
        );
        let _ = notifier.push_status(&snap, &lines).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use minewrap_core::config::{DaemonConfig, DiscordConfig, InstanceConfig};

    fn instance(id: &str, discord: DiscordConfig) -> InstanceConfig {
        InstanceConfig {
            id: id.into(),
            name: String::new(),
            work_dir: "/tmp".into(),
            java_path: "java".into(),
            jvm_args: vec![],
            launch_args: vec![],
            autostart: false,
            probe: Default::default(),
            restart: Default::default(),
            rcon: None,
            discord,
        }
    }

    fn webhook_config(url: String) -> DiscordConfig {
        DiscordConfig {
            enabled: true,
            webhook_url: Some(url),
            mode: "new-message".into(),
            interval_seconds: 3600.0,
            first_push_delay_seconds: 0.0,
            state_change_debounce_seconds: 0.0,
            state_change_posts: true,
            ..Default::default()
        }
    }

    fn sc(id: &str, old: &str, new: &str) -> minewrap_core::StateChange {
        minewrap_core::StateChange {
            instance_id: id.into(),
            old_state: old.into(),
            new_state: new.into(),
            reason: "test".into(),
        }
    }

    /// Mock webhook + manager + keeper: assert the service pushes an
    /// embed on a state change and a periodic status while online.
    #[tokio::test]
    async fn state_change_and_periodic_push_reach_webhook() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let hits = Arc::new(AtomicUsize::new(0));
        let hits2 = hits.clone();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                let _ = sock.read(&mut buf).await;
                hits2.fetch_add(1, Ordering::SeqCst);
                let body = br#"{"id":"mock-message"}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.write_all(body).await;
            }
        });

        let cfg = DaemonConfig {
            instances: vec![instance(
                "d1",
                webhook_config(format!("http://127.0.0.1:{port}/api/webhooks/1/x")),
            )],
            ..Default::default()
        };
        let mgr = Arc::new(InstanceManager::load(
            cfg,
            Arc::new(minewrap_core::EventBus::new()),
            None,
        ));
        let _keeper = crate::state_keeper::spawn(mgr.clone());

        // State change -> embed POST (+ status push, mode new-message).
        // debounce 0 => leading-edge immediate push.
        let mut per: HashMap<String, PerInstance> = HashMap::new();
        handle_state_change(&mgr, &mut per, sc("d1", "offline", "online"));
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(
            hits.load(Ordering::SeqCst) >= 1,
            "state-change push must reach the webhook"
        );

        // Periodic: first_push_delay 0 + due => a status push. Fold the
        // instance to online through the keeper first (fire_due only
        // pushes while the merged state is online).
        mgr.bus()
            .publish(&Event::StateChange(sc("d1", "offline", "starting")));
        mgr.bus()
            .publish(&Event::StateChange(sc("d1", "starting", "online")));
        tokio::time::sleep(Duration::from_millis(150)).await;
        fire_due(&mgr, &mut per);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(
            hits.load(Ordering::SeqCst) >= 2,
            "periodic push must reach the webhook while online"
        );
        server.abort();
    }

    #[test]
    fn disabled_discord_is_skipped() {
        let cfg = DaemonConfig {
            instances: vec![instance("d2", DiscordConfig::default())],
            ..Default::default()
        };
        let mgr = Arc::new(InstanceManager::load(
            cfg,
            Arc::new(minewrap_core::EventBus::new()),
            None,
        ));
        let mut per = HashMap::new();
        handle_state_change(&mgr, &mut per, sc("d2", "offline", "online"));
        assert!(per.is_empty(), "disabled instance must not be scheduled");
    }
}
