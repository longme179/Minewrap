//! `NodeService`: routes IPC calls by `node_id` to the embedded
//! local core `ProcessManager` (or, in Phase 5, to `minewrap-remote`).
//! There is no local/remote branch in UI code and tokens never enter
//! the webview.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use minewrap_core::{
    Control, DaemonConfig, EventBus, InstanceConfig, InstanceState, InstanceSummary,
    LogBuffer, LogLine, ProcessManager, ProcessState, ProbeInfo, PropertiesDocument,
    mods, properties,
};

use crate::types::{LOCAL_NODE_ID, MinewrapError, NodeState, NodeSummary};

/// Per-instance runtime: merged state + process owner + log ring.
pub struct InstanceRuntime {
    pub id: String,
    pub name: String,
    pub config: InstanceConfig,
    pub state: InstanceState,
    /// Owner task handle (`None` until `ProcessManager::spawn` runs).
    pub handle: Option<minewrap_core::OwnerHandle>,
    pub buffer: Arc<Mutex<LogBuffer>>,
    pub probe: ProbeInfo,
}

#[derive(Default)]
pub struct NodeServiceInner {
    pub nodes: HashMap<String, NodeSummary>,
    pub instances: HashMap<String, HashMap<String, InstanceRuntime>>,
}

/// Load a config, spawn one `ProcessManager` per instance, expose the
/// IPC surface. The owner tasks run on the caller's tokio runtime
/// (Tauri `async` runtime, or `minewrapd`'s).
pub struct NodeService {
    pub inner: Mutex<NodeServiceInner>,
    pub bus: Arc<EventBus>,
    pub daemon: DaemonConfig,
    pub config_path: String,
}

impl NodeService {
    pub fn load(config_path: &str) -> Result<Self, MinewrapError> {
        let daemon = DaemonConfig::load_from(config_path)
            .map_err(|e| MinewrapError::invalid(format!("config: {e}")))?;
        let bus = Arc::new(EventBus::new());
        let svc = Self {
            inner: Mutex::new(NodeServiceInner::default()),
            bus: bus.clone(),
            daemon,
            config_path: config_path.to_string(),
        };

        svc.with_inner(|inner| {
            inner.nodes.insert(
                LOCAL_NODE_ID.to_string(),
                NodeSummary {
                    id: LOCAL_NODE_ID.to_string(),
                    name: "This machine".to_string(),
                    url: "local".to_string(),
                    is_local: true,
                    state: NodeState::Connected,
                },
            );
        });

        let configs = svc.daemon.instances.clone();
        svc.register_instances(&configs);
        Ok(svc)
    }

    pub fn from_config(daemon: DaemonConfig, bus: Arc<EventBus>) -> Self {
        let svc = Self {
            inner: Mutex::new(NodeServiceInner::default()),
            bus,
            config_path: String::new(),
            daemon,
        };
        svc.with_inner(|inner| {
            inner.nodes.insert(
                LOCAL_NODE_ID.to_string(),
                NodeSummary {
                    id: LOCAL_NODE_ID.to_string(),
                    name: "This machine".to_string(),
                    url: "local".to_string(),
                    is_local: true,
                    state: NodeState::Connected,
                },
            );
        });
        let configs = svc.daemon.instances.clone();
        svc.register_instances(&configs);
        svc
    }

    fn register_instances(&self, configs: &[InstanceConfig]) {
        let defaults = self.daemon.daemon.defaults.clone();
        for cfg in configs {
            let buf = Arc::new(Mutex::new(LogBuffer::new(defaults.log_buffer_lines)));
            let mgr = ProcessManager::new(
                cfg.clone(),
                defaults.clone(),
                self.bus.clone(),
                buf.clone(),
                None, // RotatingFile tee lands in Phase 6 (file persistence)
            );
            let handle = mgr.spawn();
            let state = InstanceState::new(
                cfg.id.clone(),
                cfg.display_name().to_string(),
                self.daemon.daemon.unresponsive_after_failures,
            );
            let rt = InstanceRuntime {
                id: cfg.id.clone(),
                name: cfg.display_name().to_string(),
                config: cfg.clone(),
                state,
                handle: Some(handle),
                buffer: buf,
                probe: ProbeInfo::default(),
            };
            self.with_inner(|inner| {
                inner
                    .instances
                    .entry(LOCAL_NODE_ID.to_string())
                    .or_default()
                    .insert(rt.id.clone(), rt);
            });
        }
    }

    pub fn with_inner<R>(&self, f: impl FnOnce(&mut NodeServiceInner) -> R) -> R {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut guard)
    }

    pub fn is_local(&self, node_id: &str) -> bool {
        node_id == LOCAL_NODE_ID
    }

    pub fn remote_offline(&self, node_id: &str) -> MinewrapError {
        MinewrapError::offline(format!(
            "remote node {node_id} is not reachable (Phase 5: minewrap-remote client)"
        ))
    }

    pub fn list_nodes(&self) -> Result<Vec<NodeSummary>, MinewrapError> {
        Ok(self.with_inner(|inner| inner.nodes.values().cloned().collect()))
    }

    pub fn list_instances(&self, node_id: &str) -> Result<Vec<InstanceSummary>, MinewrapError> {
        if !self.is_local(node_id) {
            return Err(self.remote_offline(node_id));
        }
        Ok(self.with_inner(|inner| {
            inner
                .instances
                .get(node_id)
                .map(|m| m.values().map(|rt| rt.state.summary()).collect())
                .unwrap_or_default()
        }))
    }

    pub fn snapshot(&self, node_id: &str, instance_id: &str) -> Result<SnapshotDto, MinewrapError> {
        if !self.is_local(node_id) {
            return Err(self.remote_offline(node_id));
        }
        Ok(self.with_inner(|inner| {
            let rt = match inner
                .instances
                .get(node_id)
                .and_then(|m| m.get(instance_id))
            {
                Some(rt) => rt,
                None => {
                    return SnapshotDto {
                        id: instance_id.to_string(),
                        name: instance_id.to_string(),
                        combined_state: "offline".to_string(),
                        process_state: "offline".to_string(),
                        probe_ok: false,
                        players_online: None,
                        players_max: None,
                        version: None,
                    }
                }
            };
            let snap = rt.state.snapshot();
            SnapshotDto {
                id: snap.id,
                name: snap.name,
                combined_state: snap.combined_state,
                process_state: match snap.process.state {
                    ProcessState::Online => "online".to_string(),
                    ProcessState::Starting => "starting".to_string(),
                    ProcessState::Stopping => "stopping".to_string(),
                    ProcessState::Crashed => "crashed".to_string(),
                    ProcessState::Offline => "offline".to_string(),
                },
                probe_ok: snap.probe.ok,
                players_online: snap.probe.players_online,
                players_max: snap.probe.players_max,
                version: snap.probe.version,
            }
        }))
    }

    pub fn register_probe(&self, instance_id: &str, probe: ProbeInfo) {
        if let Ok(mut guard) = self.inner.lock() {
            if let Some(rt) = guard
                .instances
                .get_mut(LOCAL_NODE_ID)
                .and_then(|m| m.get_mut(instance_id))
            {
                rt.state.update_probe(probe.clone());
                rt.probe = probe;
            }
        }
    }

    pub fn backfill_logs(
        &self,
        node_id: &str,
        instance_id: &str,
        limit: usize,
        since_seq: u64,
    ) -> Result<Vec<LogLine>, MinewrapError> {
        if !self.is_local(node_id) {
            return Err(self.remote_offline(node_id));
        }
        let buf = self
            .with_inner(|inner| {
                inner
                    .instances
                    .get(LOCAL_NODE_ID)
                    .and_then(|m| m.get(instance_id))
                    .map(|rt| rt.buffer.clone())
            })
            .ok_or_else(|| MinewrapError::gone(format!("instance {instance_id} not found")))?;
        let guard = buf.lock().unwrap_or_else(|e| e.into_inner());
        let since = guard.since(since_seq);
        if since.is_empty() {
            Ok(guard.tail(limit))
        } else {
            Ok(since)
        }
    }

    pub fn work_dir(&self, instance_id: &str) -> Result<std::path::PathBuf, MinewrapError> {
        Ok(std::path::PathBuf::from(self.config_of(instance_id)?.work_dir))
    }

    pub fn config_of(&self, instance_id: &str) -> Result<InstanceConfig, MinewrapError> {
        self.with_inner(|inner| {
            inner
                .instances
                .get(LOCAL_NODE_ID)
                .and_then(|m| m.get(instance_id))
                .map(|rt| rt.config.clone())
                .ok_or_else(|| MinewrapError::gone(format!("instance {instance_id} not found")))
        })
    }

    // --- lifecycle (real `ProcessManager::Control` mux) ---

    pub fn start(&self, node_id: &str, instance_id: &str) -> Result<bool, MinewrapError> {
        if !self.is_local(node_id) {
            return Err(self.remote_offline(node_id));
        }
        let tx = self.ctrl_tx(instance_id)?;
        tokio::spawn(async move {
            let _ = tx.send(Control::Start).await;
        });
        Ok(true)
    }

    pub fn stop(&self, node_id: &str, instance_id: &str) -> Result<bool, MinewrapError> {
        if !self.is_local(node_id) {
            return Err(self.remote_offline(node_id));
        }
        let tx = self.ctrl_tx(instance_id)?;
        tokio::spawn(async move {
            let _ = tx.send(Control::Stop).await;
        });
        Ok(true)
    }

    pub fn restart(&self, node_id: &str, instance_id: &str) -> Result<bool, MinewrapError> {
        if !self.is_local(node_id) {
            return Err(self.remote_offline(node_id));
        }
        self.stop(node_id, instance_id)?;
        let tx = self.ctrl_tx(instance_id)?;
        tokio::spawn(async move {
            let _ = tx.send(Control::Start).await;
        });
        Ok(true)
    }

    pub fn send_command(
        &self,
        node_id: &str,
        instance_id: &str,
        command: &str,
    ) -> Result<bool, MinewrapError> {
        if !self.is_local(node_id) {
            return Err(self.remote_offline(node_id));
        }
        let tx = self.ctrl_tx(instance_id)?;
        let cmd = command.trim().to_string();
        if cmd.contains('\n') || cmd.contains('\r') {
            return Err(MinewrapError::invalid("command must be a single line"));
        }
        tokio::spawn(async move {
            let _ = tx.send(Control::SendCommand(cmd)).await;
        });
        Ok(true)
    }

    fn ctrl_tx(&self, instance_id: &str) -> Result<tokio::sync::mpsc::Sender<Control>, MinewrapError> {
        self.with_inner(|inner| match inner.instances.get(LOCAL_NODE_ID).and_then(|m| m.get(instance_id)) {
            Some(rt) => rt
                .handle
                .as_ref()
                .map(|h| h.ctrl_tx.clone())
                .ok_or_else(|| MinewrapError::gone("owner task not started")),
            None => Err(MinewrapError::gone(format!("instance {instance_id} not found"))),
        })
    }

    // --- properties / mods (real core calls, confined) ---

    pub fn read_properties(&self, instance_id: &str) -> Result<(String, Vec<String>), MinewrapError> {
        let dir = self.work_dir(instance_id)?;
        let path = properties::properties_path(Path::new(&dir));
        let text = std::fs::read_to_string(&path)
            .map_err(|e| MinewrapError::gone(format!("server.properties: {e}")))?;
        let doc = PropertiesDocument::parse(&text);
        Ok((doc.to_string(), Vec::new()))
    }

    pub fn write_properties(&self, instance_id: &str, text: &str) -> Result<bool, MinewrapError> {
        let dir = self.work_dir(instance_id)?;
        let path = properties::properties_path(Path::new(&dir));
        let doc = PropertiesDocument::parse(text);
        let mut errors = Vec::new();
        for line in &doc.lines {
            if let minewrap_core::properties::Line::Property(entry) = line {
                if let Err(e) = minewrap_core::validate_property(&entry.key, &entry.value) {
                    errors.push(format!("{}: {}", entry.key, e));
                }
            }
        }
        if !errors.is_empty() {
            return Err(MinewrapError::invalid(errors.join("; ")));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| MinewrapError::invalid(e.to_string()))?;
        }
        let tmp = path.with_extension("properties.tmp");
        std::fs::write(&tmp, text).map_err(|e| MinewrapError::invalid(e.to_string()))?;
        std::fs::rename(&tmp, &path).map_err(|e| MinewrapError::invalid(e.to_string()))?;
        Ok(true)
    }

    pub fn list_mods(&self, instance_id: &str) -> Result<Vec<crate::types::ModDto>, MinewrapError> {
        let dir = self.work_dir(instance_id)?;
        let entries =
            mods::list_mods(Path::new(&dir)).map_err(|e| MinewrapError::invalid(e.to_string()))?;
        Ok(entries
            .into_iter()
            .map(|e| crate::types::ModDto {
                name: e.name,
                size: e.size,
                enabled: e.enabled,
            })
            .collect())
    }

    pub fn toggle_mod(&self, instance_id: &str, name: &str) -> Result<bool, MinewrapError> {
        let dir = self.work_dir(instance_id)?;
        Ok(mods::toggle_mod(Path::new(&dir), name).map_err(|e| MinewrapError::invalid(e.to_string()))?)
    }

    /// Subscribe to the shared event bus (returns the receiver; the
    /// caller spawns a forwarder that emits `logs:batch` / `state:changed`).
    pub fn subscribe(&self) -> minewrap_core::events::Subscription {
        self.bus.subscribe(minewrap_core::events::WILDCARD)
    }
}

/// Flattened snapshot DTO (process state flattened to a string).
#[derive(Debug, Clone, serde::Serialize)]
pub struct SnapshotDto {
    pub id: String,
    pub name: String,
    pub combined_state: String,
    pub process_state: String,
    pub probe_ok: bool,
    pub players_online: Option<u32>,
    pub players_max: Option<u32>,
    pub version: Option<String>,
}
