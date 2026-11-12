//! Background SLP poller (task B2, gap G3): one shared "status loop"
//! that probes every running instance each `status_poll_interval`
//! seconds and folds results into `InstanceState`.
//!
//! Design notes (vs. v1.9.3):
//! - A single shared loop (not one `Poller` per instance) so
//!   add/remove between ticks is picked up naturally.
//! - Only `starting`/`online`/`stopping` instances are probed: SLP
//!   against a stopped server is guaranteed-fail noise.
//! - `unresponsive` is derived state (`combined_state()` after
//!   `unresponsive_after_failures` failed probes) — no synthetic
//!   `StateChange` events are published for it.

use std::sync::Arc;
use std::time::Duration;

use futures_util::future::join_all;
use minewrap_core::{ProcessState, probe_once};

use crate::instances::InstanceManager;

/// Spawn the poller task. Runs for the lifetime of the daemon.
pub fn spawn(mgr: Arc<InstanceManager>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let interval = {
            let daemon = mgr.daemon.lock().unwrap_or_else(|e| e.into_inner());
            daemon.daemon.status_poll_interval.max(0.1)
        };
        let mut tick = tokio::time::interval(Duration::from_secs_f64(interval));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            poll_once_all(&mgr).await;
        }
    })
}

/// One parallel probe sweep over all running instances.
async fn poll_once_all(mgr: &InstanceManager) {
    let targets: Vec<(String, minewrap_core::ProbeConfig)> = mgr
        .probe_targets()
        .into_iter()
        .filter(|(id, _)| is_running(mgr, id))
        .collect();
    if targets.is_empty() {
        return;
    }
    let results = join_all(targets.iter().map(|(_, cfg)| probe_once(cfg))).await;
    for ((id, _), info) in targets.into_iter().zip(results) {
        mgr.apply_probe(&id, info);
    }
}

fn is_running(mgr: &InstanceManager, id: &str) -> bool {
    matches!(
        mgr.snapshot_of(id).map(|s| s.process.state),
        Some(ProcessState::Starting) | Some(ProcessState::Online) | Some(ProcessState::Stopping)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use minewrap_core::config::{DaemonConfig, InstanceConfig};
    use minewrap_core::events::EventBus;

    fn instance(id: &str, port: u16) -> InstanceConfig {
        InstanceConfig {
            id: id.into(),
            name: String::new(),
            work_dir: "/tmp".into(),
            java_path: "java".into(),
            jvm_args: vec![],
            launch_args: vec![],
            autostart: false,
            probe: minewrap_core::ProbeConfig {
                host: "127.0.0.1".into(),
                port,
                query_enabled: false,
                query_port: None,
                timeout: 0.5,
            },
            restart: Default::default(),
            rcon: None,
            discord: Default::default(),
        }
    }

    #[tokio::test]
    async fn offline_instances_are_not_probed_and_unresponsive_derives() {
        let cfg = DaemonConfig {
            daemon: minewrap_core::DaemonSettings {
                unresponsive_after_failures: 2,
                ..Default::default()
            },
            instances: vec![instance("idle", 25565)],
        };
        let mgr = Arc::new(InstanceManager::load(cfg, Arc::new(EventBus::new()), None));
        let _keeper = crate::state_keeper::spawn(mgr.clone());

        // Offline: a sweep must not touch the probe state.
        poll_once_all(&mgr).await;
        assert!(mgr.snapshot_of("idle").unwrap().probe.checked_at.is_none());

        // Simulate a running process whose SLP port is dead: after 2
        // failed probes the combined state flips to `unresponsive`.
        for state in ["starting", "online"] {
            mgr.bus().publish(&minewrap_core::Event::StateChange(
                minewrap_core::StateChange {
                    instance_id: "idle".into(),
                    old_state: "offline".into(),
                    new_state: state.into(),
                    reason: "test".into(),
                },
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(mgr.snapshot_of("idle").unwrap().combined_state, "online");

        poll_once_all(&mgr).await;
        poll_once_all(&mgr).await;
        let snap = mgr.snapshot_of("idle").unwrap();
        assert_eq!(
            snap.combined_state, "unresponsive",
            "2 probe failures => unresponsive"
        );
        assert!(snap.probe.error.is_some());

        // A successful probe resets the streak (fake SLP on a socket
        // that accepts but speaks nothing still fails; use a real
        // listener below only for the ok-path).
    }

    #[tokio::test]
    async fn successful_probe_marks_online_and_fills_fields() {
        // Minimal fake SLP server: accept + respond with a framed
        // status JSON (enough for parse_status_json).
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 512];
                let _ = sock.read(&mut buf).await;
                let json = br#"{"version":{"name":"1.20.1","protocol":763},"players":{"max":20,"online":2},"description":{"text":"A Server"}}"#;
                let mut payload = vec![0x00u8];
                let mut len = vec![];
                let mut n = json.len();
                loop {
                    let mut b = (n & 0x7F) as u8;
                    n >>= 7;
                    if n != 0 {
                        b |= 0x80;
                    }
                    len.push(b);
                    if n == 0 {
                        break;
                    }
                }
                payload.extend_from_slice(&len);
                payload.extend_from_slice(json);
                let mut frame = vec![];
                let mut pn = payload.len();
                loop {
                    let mut b = (pn & 0x7F) as u8;
                    pn >>= 7;
                    if pn != 0 {
                        b |= 0x80;
                    }
                    frame.push(b);
                    if pn == 0 {
                        break;
                    }
                }
                frame.extend_from_slice(&payload);
                // The client reads `length + 5` bytes; pad so the read
                // is satisfied before the socket closes.
                frame.extend_from_slice(&[0u8; 8]);
                let _ = sock.write_all(&frame).await;
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        });

        let cfg = DaemonConfig {
            instances: vec![instance("live", port)],
            ..Default::default()
        };
        let mgr = Arc::new(InstanceManager::load(cfg, Arc::new(EventBus::new()), None));
        let _keeper = crate::state_keeper::spawn(mgr.clone());
        mgr.bus().publish(&minewrap_core::Event::StateChange(
            minewrap_core::StateChange {
                instance_id: "live".into(),
                old_state: "offline".into(),
                new_state: "online".into(),
                reason: "test".into(),
            },
        ));
        tokio::time::sleep(Duration::from_millis(100)).await;

        poll_once_all(&mgr).await;
        let _ = server.await;
        let snap = mgr.snapshot_of("live").unwrap();
        assert_eq!(snap.combined_state, "online");
        assert_eq!(snap.probe.players_online, Some(2));
        assert_eq!(snap.probe.version.as_deref(), Some("1.20.1"));
        assert!(snap.probe.latency_ms.is_some());
    }
}
