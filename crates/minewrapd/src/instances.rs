//! Instance manager: builds and owns per-instance runtime objects
//! (port of v1 `instances.py`'s `ManagedInstance` + `InstanceManager`).
//!
//! Unlike v1 — which imports the Python daemon stack — this crate is
//! the headless Axum service; the Tauri shell carries its own
//! `node_service.rs` (`src-tauri`). Both share `minewrap-core` and
//! identically-shaped routes/WS messages.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use minewrap_core::{
    Control, DaemonConfig, Event, EventBus, InstanceConfig, InstanceState, LogBuffer, ProbeConfig,
    ProbeInfo, ProcessManager,
};

/// Per-instance runtime (parity with v1 `ManagedInstance`).
pub struct ManagedInstance {
    pub config: InstanceConfig,
    pub buffer: Arc<Mutex<LogBuffer>>,
    pub state: InstanceState,
    pub handle: Option<minewrap_core::OwnerHandle>,
    pub probe: ProbeInfo,
    /// Last failure reason (spawn_failed / crash tail), surfaced in
    /// the snapshot JSON as `last_error` (GUI parity).
    pub last_error: Option<String>,
}

/// Owns every instance + the shared bus; request handlers go through it.
pub struct InstanceManager {
    pub instances: Mutex<HashMap<String, ManagedInstance>>,
    pub bus: Arc<EventBus>,
    pub daemon: Mutex<DaemonConfig>,
    pub config_path: Option<String>,
    /// Structural-change notifications (instance added/removed) for
    /// the `/events` WebSocket.
    changes: tokio::sync::broadcast::Sender<()>,
}

impl InstanceManager {
    pub fn load(config: DaemonConfig, bus: Arc<EventBus>, config_path: Option<String>) -> Self {
        let (changes, _) = tokio::sync::broadcast::channel(8);
        let mut mgr = Self {
            instances: Mutex::new(HashMap::new()),
            bus: bus.clone(),
            daemon: Mutex::new(config),
            config_path,
            changes,
        };
        mgr.start_all();
        mgr
    }

    fn start_all(&mut self) {
        let (defaults, configs, unresponsive_after_failures) = {
            let daemon = self.daemon.lock().unwrap_or_else(|e| e.into_inner());
            (
                daemon.daemon.defaults.clone(),
                daemon.instances.clone(),
                daemon.daemon.unresponsive_after_failures,
            )
        };
        for cfg in configs {
            let autostart = cfg.autostart;
            let id = cfg.id.clone();
            let buf = Arc::new(Mutex::new(LogBuffer::new(defaults.log_buffer_lines)));
            let state = InstanceState::new(
                cfg.id.clone(),
                cfg.display_name().to_string(),
                unresponsive_after_failures,
            );
            // D2-lite (task B8): owner tasks are spawned lazily — only
            // `autostart` instances get one (and a Start) at load; the
            // rest stay config-only until their first `start` (saves
            // the old +0.75 MiB-per-offline-instance owner cost).
            self.instances
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(
                    cfg.id.clone(),
                    ManagedInstance {
                        config: cfg,
                        buffer: buf,
                        state,
                        handle: None,
                        probe: ProbeInfo::default(),
                        last_error: None,
                    },
                );
            if autostart {
                self.ensure_runtime(&id);
                self.send_control(&id, Control::Start);
            }
        }
    }

    /// Spawn the owner task for an instance on demand (D2-lite).
    /// Returns false when the id is unknown.
    pub fn ensure_runtime(&self, id: &str) -> bool {
        let needs_spawn = {
            let guard = self.instances.lock().unwrap_or_else(|e| e.into_inner());
            guard.get(id).map(|m| m.handle.is_none()).unwrap_or(false)
        };
        if !needs_spawn {
            return true;
        }
        let (config, defaults, unresponsive_after_failures, buffer) = {
            let guard = self.instances.lock().unwrap_or_else(|e| e.into_inner());
            let Some(m) = guard.get(id) else { return false };
            let defaults = self
                .daemon
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .daemon
                .defaults
                .clone();
            let unresponsive = self
                .daemon
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .daemon
                .unresponsive_after_failures;
            (m.config.clone(), defaults, unresponsive, m.buffer.clone())
        };
        let manager = ProcessManager::new(config, defaults, self.bus.clone(), buffer, None);
        let handle = manager.spawn();
        let mut guard = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        match guard.get_mut(id) {
            Some(m) => {
                m.handle = Some(handle);
                m.state = InstanceState::new(
                    id.to_string(),
                    m.config.display_name().to_string(),
                    unresponsive_after_failures,
                );
                m.last_error = None;
                true
            }
            None => false,
        }
    }

    /// Start an instance (lazy-safe: spawns the owner task first).
    pub fn start_instance(&self, id: &str) -> bool {
        if !self.ensure_runtime(id) {
            return false;
        }
        self.send_control(id, Control::Start)
    }

    /// Restart: graceful stop, wait for the state to settle, then a
    /// fresh start (the v1.9.3 daemon fired Start while still
    /// `stopping`, which the state machine ignored).
    pub async fn restart_instance(&self, id: &str) -> bool {
        if self.config_of(id).is_none() {
            return false;
        }
        let (stop_timeout, kill_grace) = {
            let daemon = self.daemon.lock().unwrap_or_else(|e| e.into_inner());
            (
                daemon.daemon.defaults.stop_timeout,
                daemon.daemon.defaults.kill_grace_seconds,
            )
        };
        self.send_control(id, Control::Stop);
        let deadline = tokio::time::Instant::now()
            + std::time::Duration::from_secs_f64(stop_timeout + kill_grace + 2.0);
        loop {
            let settled = matches!(
                self.snapshot_of(id).map(|s| s.process.state),
                None | Some(minewrap_core::ProcessState::Offline)
                    | Some(minewrap_core::ProcessState::Crashed)
            );
            if settled || tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        self.start_instance(id)
    }

    pub fn count(&self) -> usize {
        self.instances
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }

    pub fn ids(&self) -> Vec<String> {
        self.instances
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    pub fn runtime(
        &self,
        _id: &str,
    ) -> Option<std::sync::MutexGuard<'_, HashMap<String, ManagedInstance>>> {
        self.instances.lock().ok()
    }

    pub fn config_of(&self, id: &str) -> Option<InstanceConfig> {
        self.instances
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .map(|m| m.config.clone())
    }

    /// Patch the in-memory config + persist to the daemon TOML, then
    /// push the new config to the running process manager so the next
    /// start uses it.
    pub fn patch_instance(&self, id: &str, patch: serde_json::Value) -> Result<(), String> {
        let cfg = self
            .config_of(id)
            .ok_or_else(|| format!("unknown instance: {id}"))?;
        // Merge into a json doc for core::patch_instance reuse.
        let mut doc = serde_json::json!({
            "name": cfg.name,
            "work_dir": cfg.work_dir,
            "java_path": cfg.java_path,
            "jvm_args": cfg.jvm_args,
            "launch_args": cfg.launch_args,
            "autostart": cfg.autostart,
            "probe": serde_json::to_value(&cfg.probe).unwrap_or(serde_json::json!({})),
            "restart": serde_json::to_value(&cfg.restart).unwrap_or(serde_json::json!({})),
            "discord": serde_json::to_value(&cfg.discord).unwrap_or(serde_json::json!({})),
        });
        for (k, v) in patch.as_object().cloned().unwrap_or_default() {
            doc[&k] = v;
        }

        let updated = {
            let mut daemon = self.daemon.lock().unwrap_or_else(|e| e.into_inner());
            daemon.patch_instance(id, doc)?;
            let updated = daemon
                .instances
                .iter()
                .find(|c| c.id == id)
                .cloned()
                .ok_or_else(|| format!("unknown instance: {id}"))?;
            if let Some(path) = &self.config_path {
                daemon.save_to(path)?;
            }
            updated
        };

        {
            let mut guard = self.instances.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(m) = guard.get_mut(id) {
                m.config = updated.clone();
            }
        }
        self.send_control(id, Control::ApplyConfig(updated));
        Ok(())
    }

    pub fn summary_of(&self, id: &str) -> Option<minewrap_core::InstanceSummary> {
        let guard = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        guard.get(id).map(|m| {
            // `config` is the source of truth for the display name:
            // `patch_instance` already wrote the new name there; the state
            // cache (`m.state.name`) is only rebuilt at spawn, so without
            // this override a renamed instance keeps showing its old name
            // until the daemon restarts (BUG-D1).
            let mut s = m.state.summary();
            s.name = m.config.display_name().to_string();
            s
        })
    }

    pub fn snapshot_of(&self, id: &str) -> Option<minewrap_core::InstanceSnapshot> {
        let guard = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        guard.get(id).map(|m| {
            let mut s = m.state.snapshot();
            s.name = m.config.display_name().to_string();
            s
        })
    }

    /// Snapshot as JSON with the daemon-additive `last_error` field
    /// (GUI `SnapshotDto` parity; additive, API stays "1.0").
    pub fn snapshot_json(&self, id: &str) -> Option<serde_json::Value> {
        let guard = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        let m = guard.get(id)?;
        let mut v = serde_json::to_value(m.state.snapshot()).unwrap_or(serde_json::json!({}));
        if let Some(obj) = v.as_object_mut() {
            // Same source-of-truth override as snapshot_of (name is not
            // rebuilt in the state cache until the next spawn).
            obj.insert("name".into(), serde_json::json!(m.config.display_name()));
            obj.insert("last_error".into(), serde_json::json!(m.last_error));
        }
        Some(v)
    }

    pub fn last_error_of(&self, id: &str) -> Option<String> {
        let guard = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        guard.get(id).and_then(|m| m.last_error.clone())
    }

    /// `(id, ProbeConfig)` pairs for the background SLP poller.
    pub fn probe_targets(&self) -> Vec<(String, ProbeConfig)> {
        let guard = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .iter()
            .map(|(id, m)| (id.clone(), m.config.probe.clone()))
            .collect()
    }

    /// Apply a fresh probe result (poller task T2).
    pub fn apply_probe(&self, id: &str, info: ProbeInfo) {
        let mut guard = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(m) = guard.get_mut(id) {
            m.state.update_probe(info.clone());
            m.probe = info;
        }
    }

    /// Add an instance at runtime (validates, spawns its owner task,
    /// persists to TOML). The caller is responsible for the duplicate
    /// check (HTTP 409).
    pub fn add_instance(&self, cfg: InstanceConfig) -> Result<(), String> {
        cfg.validate()?;
        let (defaults, unresponsive_after_failures) = {
            let daemon = self.daemon.lock().unwrap_or_else(|e| e.into_inner());
            (
                daemon.daemon.defaults.clone(),
                daemon.daemon.unresponsive_after_failures,
            )
        };
        let buf = Arc::new(Mutex::new(LogBuffer::new(defaults.log_buffer_lines)));
        let state = InstanceState::new(
            cfg.id.clone(),
            cfg.display_name().to_string(),
            unresponsive_after_failures,
        );
        let autostart = cfg.autostart;
        let id = cfg.id.clone();

        {
            let mut daemon = self.daemon.lock().unwrap_or_else(|e| e.into_inner());
            daemon.instances.push(cfg.clone());
            daemon.validate()?;
            if let Some(path) = &self.config_path {
                daemon.save_to(path)?;
            }
        }
        self.instances
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                id.clone(),
                ManagedInstance {
                    config: cfg,
                    buffer: buf,
                    state,
                    handle: None, // D2-lite: lazy owner task
                    probe: ProbeInfo::default(),
                    last_error: None,
                },
            );
        if autostart {
            self.ensure_runtime(&id);
            self.send_control(&id, Control::Start);
        }
        self.notify_changed();
        Ok(())
    }

    /// Finish removing an instance after the caller has stopped it:
    /// drop the runtime, persist the TOML without it. Returns false
    /// when the id was unknown.
    pub fn finish_remove_instance(&self, id: &str) -> bool {
        let removed = {
            let mut guard = self.instances.lock().unwrap_or_else(|e| e.into_inner());
            guard.remove(id).is_some()
        };
        if !removed {
            return false;
        }
        {
            let mut daemon = self.daemon.lock().unwrap_or_else(|e| e.into_inner());
            daemon.instances.retain(|c| c.id != id);
            if let Some(path) = &self.config_path {
                let _ = daemon.save_to(path);
            }
        }
        self.notify_changed();
        true
    }

    /// Subscribe to structural-change notifications (add/remove).
    pub fn subscribe_changes(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.changes.subscribe()
    }

    fn notify_changed(&self) {
        let _ = self.changes.send(());
    }

    pub fn backfill(&self, id: &str, limit: usize, since_seq: u64) -> Vec<minewrap_core::LogLine> {
        let guard = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        let m = match guard.get(id) {
            Some(m) => m,
            None => return Vec::new(),
        };
        let buf = m.buffer.lock().unwrap_or_else(|e| e.into_inner());
        let since = buf.since(since_seq);
        if since.is_empty() {
            buf.tail(limit)
        } else {
            since
        }
    }

    pub fn data_dir(&self) -> PathBuf {
        let daemon = self.daemon.lock().unwrap_or_else(|e| e.into_inner());
        PathBuf::from(&daemon.daemon.data_dir)
    }

    pub fn work_dir_of(&self, id: &str) -> Option<PathBuf> {
        self.config_of(id).map(|c| PathBuf::from(c.work_dir))
    }

    pub fn send_control(&self, id: &str, control: Control) -> bool {
        let guard = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        let m = match guard.get(id) {
            Some(m) => m,
            None => return false,
        };
        let tx = match m.handle.as_ref() {
            Some(h) => h.ctrl_tx.clone(),
            None => return false,
        };
        drop(guard);
        tokio::spawn(async move {
            let _ = tx.send(control).await;
        });
        true
    }

    pub fn subscribe(&self, topic: &str) -> minewrap_core::events::Subscription {
        self.bus.subscribe(topic)
    }

    pub fn bus(&self) -> Arc<EventBus> {
        self.bus.clone()
    }
}

/// Join a data-dir-relative path; absolute paths are used as-is.
pub fn under_data_dir(data_dir: &Path, rel: &str) -> PathBuf {
    if Path::new(rel).is_absolute() {
        PathBuf::from(rel)
    } else {
        data_dir.join(rel)
    }
}

#[allow(dead_code)]
fn _unused(_: &Event) {}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn add_and_remove_instance_persist_round_trip() {
        let dir = std::env::temp_dir().join(format!("minewrapd-mgr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("minewrap.toml");
        let mut cfg = DaemonConfig::default();
        cfg.daemon.data_dir = dir.join("data").to_string_lossy().to_string();
        {
            let mut first = cfg.clone();
            first.instances = vec![instance("keep")];
            first.save_to(path.to_str().unwrap()).unwrap();
        }

        let loaded = DaemonConfig::load_from(path.to_str().unwrap()).unwrap();
        let mgr = InstanceManager::load(
            loaded,
            Arc::new(EventBus::new()),
            Some(path.to_string_lossy().to_string()),
        );
        assert_eq!(mgr.count(), 1);

        mgr.add_instance(instance("new")).unwrap();
        assert_eq!(mgr.count(), 2);
        // Duplicate id rejected by the TOML validator.
        assert!(mgr.add_instance(instance("new")).is_err());

        assert!(mgr.finish_remove_instance("new"));
        assert!(!mgr.finish_remove_instance("ghost"));
        assert_eq!(mgr.count(), 1);

        let reloaded = DaemonConfig::load_from(path.to_str().unwrap()).unwrap();
        assert_eq!(reloaded.instances.len(), 1);
        assert_eq!(reloaded.instances[0].id, "keep");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_json_carries_last_error() {
        let mgr = InstanceManager::load(DaemonConfig::default(), Arc::new(EventBus::new()), None);
        assert!(mgr.snapshot_json("missing").is_none());
    }

    /// D2-lite (task B8): non-autostart instances load without an
    /// owner task; `start_instance` spawns it on demand; autostart
    /// instances get one immediately (observable via the spawn_failed
    /// event their bad work dir produces).
    #[tokio::test]
    async fn lazy_owner_spawn_and_autostart() {
        let mut auto = instance("auto");
        auto.autostart = true;
        auto.work_dir = "/nonexistent-minewrap-autostart-dir".into();
        let manual = instance("manual");
        let mut cfg = DaemonConfig::default();
        cfg.instances = vec![auto, manual];
        let bus = Arc::new(EventBus::new());
        let mut sub = bus.subscribe("auto");
        let mgr = InstanceManager::load(cfg, bus, None);

        // manual: no owner until the first start.
        assert!(mgr.runtime("manual").unwrap().get("manual").unwrap().handle.is_none());
        assert!(mgr.start_instance("manual"));
        assert!(mgr.runtime("manual").unwrap().get("manual").unwrap().handle.is_some());

        // auto: the owner was spawned at load and Start was sent —
        // with a broken work dir that yields the GapF same-state
        // event instead of a panic.
        let ev = tokio::time::timeout(std::time::Duration::from_secs(5), sub.rx.recv())
            .await
            .expect("autostart event within 5s")
            .expect("bus alive");
        match ev {
            minewrap_core::Event::StateChange(sc) => {
                assert!(sc.reason.contains("spawn_failed"), "reason: {}", sc.reason);
            }
            other => panic!("expected StateChange, got {other:?}"),
        }

        // start_instance on an unknown id reports false, not a panic.
        assert!(!mgr.start_instance("ghost"));
    }
}
