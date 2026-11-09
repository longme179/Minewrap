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
    Control, DaemonConfig, Event, EventBus, InstanceConfig, InstanceState, LogBuffer, ProbeInfo,
    ProcessManager,
};

/// Per-instance runtime (parity with v1 `ManagedInstance`).
pub struct ManagedInstance {
    pub config: InstanceConfig,
    pub buffer: Arc<Mutex<LogBuffer>>,
    pub state: InstanceState,
    pub handle: Option<minewrap_core::OwnerHandle>,
    pub probe: ProbeInfo,
}

/// Owns every instance + the shared bus; request handlers go through it.
pub struct InstanceManager {
    pub instances: Mutex<HashMap<String, ManagedInstance>>,
    pub bus: Arc<EventBus>,
    pub daemon: DaemonConfig,
}

impl InstanceManager {
    pub fn load(config: DaemonConfig, bus: Arc<EventBus>) -> Self {
        let mut mgr = Self {
            instances: Mutex::new(HashMap::new()),
            bus: bus.clone(),
            daemon: config,
        };
        mgr.start_all();
        mgr
    }

    fn start_all(&mut self) {
        let defaults = self.daemon.daemon.defaults.clone();
        let configs: Vec<InstanceConfig> = self.daemon.instances.clone();
        for cfg in configs {
            let buf = Arc::new(Mutex::new(LogBuffer::new(defaults.log_buffer_lines)));
            let manager = ProcessManager::new(
                cfg.clone(),
                defaults.clone(),
                self.bus.clone(),
                buf.clone(),
                None,
            );
            let handle = manager.spawn();
            let state = InstanceState::new(
                cfg.id.clone(),
                cfg.display_name().to_string(),
                self.daemon.daemon.unresponsive_after_failures,
            );
            self.instances
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(
                    cfg.id.clone(),
                    ManagedInstance {
                        config: cfg,
                        buffer: buf,
                        state,
                        handle: Some(handle),
                        probe: ProbeInfo::default(),
                    },
                );
        }
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

    pub fn runtime(&self, id: &str) -> Option<std::sync::MutexGuard<'_, HashMap<String, ManagedInstance>>> {
        self.instances.lock().ok()
    }

    pub fn config_of(&self, id: &str) -> Option<InstanceConfig> {
        self.instances
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .map(|m| m.config.clone())
    }

    pub fn summary_of(&self, id: &str) -> Option<minewrap_core::InstanceSummary> {
        let guard = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        guard.get(id).map(|m| m.state.summary())
    }

    pub fn snapshot_of(&self, id: &str) -> Option<minewrap_core::InstanceSnapshot> {
        let guard = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        guard.get(id).map(|m| m.state.snapshot())
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
        PathBuf::from(&self.daemon.daemon.data_dir)
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
