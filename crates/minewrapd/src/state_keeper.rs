//! State keeper (task B1, gap G2): one bus subscriber that folds every
//! `StateChange` into the per-instance `InstanceState` held by the
//! `InstanceManager`, so `GET /api/v1/instances` reports live states.
//!
//! This is a port of the logic the GUI ran in its event forwarder
//! (`update_process_state`, v1.9.3) —
//! same semantics (`last_error`, `started_at`/`stopped_at` inference,
//! `requested_stop`), different owner: the daemon.

use std::sync::Arc;

use minewrap_core::events::{Event, StateChange, WILDCARD};
use minewrap_core::{ProcessState, ProcessStatus};

use crate::instances::InstanceManager;

/// Spawn the keeper task. It runs for the lifetime of the daemon.
pub fn spawn(mgr: Arc<InstanceManager>) -> tokio::task::JoinHandle<()> {
    let mut sub = mgr.subscribe(WILDCARD);
    tokio::spawn(async move {
        while let Some(ev) = sub.rx.recv().await {
            if let Event::StateChange(sc) = ev {
                mgr.apply_state_change(&sc);
            }
        }
    })
}

/// Map a wire state string to the core enum (shared with tests).
fn parse_state(s: &str) -> ProcessState {
    match s {
        "online" => ProcessState::Online,
        "starting" => ProcessState::Starting,
        "stopping" => ProcessState::Stopping,
        "crashed" => ProcessState::Crashed,
        _ => ProcessState::Offline,
    }
}

impl InstanceManager {
    /// Fold one state-change event into the in-memory `InstanceState`
    /// (port of the GUI's `update_process_state`).
    pub fn apply_state_change(&self, sc: &StateChange) {
        let state = parse_state(&sc.new_state);
        let now = chrono::Utc::now();
        let mut guard = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        let Some(m) = guard.get_mut(&sc.instance_id) else {
            return;
        };
        if state == ProcessState::Crashed
            || sc.reason.contains("spawn_failed")
            || sc.reason.contains("failed")
            || sc.reason.contains("crash_during_start")
            || sc.reason.contains("startup_timeout")
        {
            m.last_error = Some(sc.reason.clone());
        } else if matches!(state, ProcessState::Starting | ProcessState::Online) {
            m.last_error = None;
        }
        let prev = m.state.snapshot().process;
        let mut status = ProcessStatus {
            state,
            pid: prev.pid,
            started_at: prev.started_at,
            stopped_at: prev.stopped_at,
            exit_code: prev.exit_code,
            requested_stop: matches!(state, ProcessState::Stopping),
            forced_stop: prev.forced_stop,
            restart_count: prev.restart_count,
            crash_count: prev.crash_count,
            uptime_seconds: None,
        };
        if matches!(state, ProcessState::Starting | ProcessState::Online)
            && status.started_at.is_none()
        {
            status.started_at = Some(now);
            status.stopped_at = None;
        }
        if matches!(state, ProcessState::Offline | ProcessState::Crashed) {
            status.stopped_at = Some(now);
            if sc.reason.contains("requested") {
                status.requested_stop = true;
            }
        }
        if state == ProcessState::Starting {
            // Fresh boot: stale probe failures must not mark the new run
            // `unresponsive` before its first probe lands.
            m.state.reset_failures();
        }
        m.state.update_process(status);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minewrap_core::config::{DaemonConfig, InstanceConfig};
    use minewrap_core::events::EventBus;

    fn manager_with(instance: InstanceConfig) -> Arc<InstanceManager> {
        let cfg = DaemonConfig {
            instances: vec![instance],
            ..Default::default()
        };
        Arc::new(InstanceManager::load(cfg, Arc::new(EventBus::new()), None))
    }

    fn instance(id: &str) -> InstanceConfig {
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
            discord: Default::default(),
        }
    }

    fn sc(id: &str, old: &str, new: &str, reason: &str) -> StateChange {
        StateChange {
            instance_id: id.into(),
            old_state: old.into(),
            new_state: new.into(),
            reason: reason.into(),
        }
    }

    #[tokio::test]
    async fn keeper_updates_snapshot_from_bus_events() {
        let mgr = manager_with(instance("t1"));
        let _keeper = spawn(mgr.clone());

        mgr.bus().publish(&Event::StateChange(sc(
            "t1",
            "offline",
            "starting",
            "start_requested",
        )));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        {
            let snap = mgr.snapshot_of("t1").unwrap();
            assert_eq!(snap.combined_state, "starting");
            assert!(snap.process.started_at.is_some());
        }

        mgr.bus().publish(&Event::StateChange(sc(
            "t1",
            "starting",
            "online",
            "done_line",
        )));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(mgr.snapshot_of("t1").unwrap().combined_state, "online");

        // Spawn failure keeps the state but records the reason (GapF shape).
        mgr.bus().publish(&Event::StateChange(sc(
            "t1",
            "online",
            "crashed",
            "crashed (exit code 1)",
        )));
        mgr.bus().publish(&Event::StateChange(sc(
            "t1",
            "crashed",
            "crashed",
            "spawn_failed: boom",
        )));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let snap = mgr.snapshot_of("t1").unwrap();
        assert_eq!(snap.combined_state, "crashed");
        assert!(
            mgr.last_error_of("t1")
                .as_deref()
                .unwrap_or("")
                .contains("spawn_failed"),
            "last_error must record spawn_failed"
        );

        // Unknown instance: silently ignored (no panic).
        mgr.bus()
            .publish(&Event::StateChange(sc("ghost", "offline", "online", "x")));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(mgr.snapshot_of("ghost").is_none());
    }
}
