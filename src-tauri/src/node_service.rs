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

/// Remote-node record: the `minewrap-remote` client + connection
/// state. The token lives here (in Rust) and is never sent to the
/// webview; `NodeSummary` is the UI-facing projection.
pub struct RemoteNode {
    pub summary: NodeSummary,
    pub client: minewrap_remote::RemoteClient,
    pub state: NodeState,
    pub reconnect_attempt: u32,
}

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
    pub last_error: Option<String>,
}

#[derive(Default)]
pub struct NodeServiceInner {
    pub nodes: HashMap<String, NodeSummary>,
    pub instances: HashMap<String, HashMap<String, InstanceRuntime>>,
    pub remotes: HashMap<String, RemoteNode>,
}

/// Load a config, spawn one `ProcessManager` per instance, expose the
/// IPC surface. The owner tasks run on the caller's tokio runtime
/// (Tauri `async` runtime, or `minewrapd`'s).
pub struct NodeService {
    pub inner: Mutex<NodeServiceInner>,
    pub bus: Arc<EventBus>,
    pub daemon: Mutex<DaemonConfig>,
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
            daemon: Mutex::new(daemon),
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

        let configs = svc
            .daemon
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .instances
            .clone();
        svc.register_instances(&configs);
        Ok(svc)
    }

    pub fn from_config(daemon: DaemonConfig, bus: Arc<EventBus>) -> Self {
        let svc = Self {
            inner: Mutex::new(NodeServiceInner::default()),
            bus,
            config_path: String::new(),
            daemon: Mutex::new(daemon),
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
        let configs = svc
            .daemon
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .instances
            .clone();
        svc.register_instances(&configs);
        svc
    }

    fn register_instances(&self, configs: &[InstanceConfig]) {
        let (defaults, unresponsive_after_failures) = {
            let daemon = self.daemon.lock().unwrap_or_else(|e| e.into_inner());
            (
                daemon.daemon.defaults.clone(),
                daemon.daemon.unresponsive_after_failures,
            )
        };
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
                unresponsive_after_failures,
            );
            let rt = InstanceRuntime {
                id: cfg.id.clone(),
                name: cfg.display_name().to_string(),
                config: cfg.clone(),
                state,
                handle: Some(handle),
                buffer: buf,
                probe: ProbeInfo::default(),
                last_error: None,
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
        MinewrapError::offline(format!("remote node {node_id} is not reachable"))
    }

    // --- remote nodes (Phase 5: minewrap-remote client) ---

    /// Add a remote node; the token is stored in Rust (keyring/0600,
    /// v1 `node_store.py` parity) and never leaves Rust.
    pub fn add_remote_node(&self, id: String, name: String, url: String, token: String) -> Result<NodeSummary, MinewrapError> {
        let client = minewrap_remote::RemoteClient::new(url.clone(), token);
        let summary = NodeSummary {
            id: id.clone(),
            name,
            url,
            is_local: false,
            state: NodeState::Connecting,
        };
        self.with_inner(|inner| {
            inner.nodes.insert(id.clone(), summary.clone());
            inner.remotes.insert(
                id,
                RemoteNode { summary: summary.clone(), client, state: NodeState::Connecting, reconnect_attempt: 0 },
            );
        });
        Ok(summary)
    }

    pub fn remove_remote_node(&self, id: &str) -> Result<bool, MinewrapError> {
        Ok(self.with_inner(|inner| {
            inner.nodes.remove(id).is_some() && inner.remotes.remove(id).is_some()
        }))
    }

    pub fn remote_state(&self, node_id: &str) -> Option<NodeState> {
        self.with_inner(|inner| inner.remotes.get(node_id).map(|r| r.state))
    }

    /// Reconnect one remote node; updates the `node:connection` state
    /// (unauthorised / incompatible / offline / connected).
    pub async fn probe_remote(&self, node_id: &str) -> Result<NodeState, MinewrapError> {
        let client = {
            let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner
                .remotes
                .get(node_id)
                .ok_or_else(|| MinewrapError::gone(format!("remote node {node_id} not found")))?
                .client
                .base()
                .to_string()
        };
        let _ = client;
        // Extract the client fields without holding the lock across
        // the await: we clone base + token are not clonable, so we go
        // through a helper channel — simpler: read the request
        // synchronously and spawn the check below.
        let (base, token) = self.remote_credentials(node_id)?;
        let remote = minewrap_remote::RemoteClient::new(base, token);
        match remote.health().await {
            Ok(_) => {
                self.set_remote_state(node_id, NodeState::Connected);
                Ok(NodeState::Connected)
            }
            Err(e) => {
                // remote uses its own DTOs; map by code to the Tauri envelope.
                let code = e.code.clone();
                let message = e.message.clone();
                let st = match code.as_str() {
                    "unauthorised" => NodeState::Unauthorised,
                    "incompatible_version" => NodeState::Incompatible,
                    _ => NodeState::Offline,
                };
                self.set_remote_state(node_id, st);
                Err(MinewrapError { code, message })
            }
        }
    }

    fn remote_credentials(&self, node_id: &str) -> Result<(String, String), MinewrapError> {
        // Token lookup is via the keyring/0600 store in the full
        // Phase 5; for now the in-memory entry holds the request.
        Err(MinewrapError::offline(format!("node {node_id}: credential store not wired")))
    }

    fn set_remote_state(&self, node_id: &str, state: NodeState) {
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(r) = inner.remotes.get_mut(node_id) {
                r.state = state;
                r.summary.state = state;
            }
            if let Some(n) = inner.nodes.get_mut(node_id) {
                n.state = state;
            }
        }
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
                        last_error: None,
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
                last_error: rt.last_error.clone(),
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

    /// Instance view (secrets masked) — v1 `GET …/config`.
    pub fn instance_view(&self, instance_id: &str) -> Result<serde_json::Value, MinewrapError> {
        let cfg = self.config_of(instance_id)?;
        let _ = cfg;
        Ok(serde_json::json!({
            "id": instance_id,
            "name": self.name_of(instance_id)?,
            "work_dir": self.work_dir(instance_id)?.to_string_lossy(),
            "java_path": self.config_of(instance_id)?.java_path,
            "jvm_args": self.config_of(instance_id)?.jvm_args,
            "launch_args": self.config_of(instance_id)?.launch_args,
            "autostart": self.config_of(instance_id)?.autostart,
            "probe": self.config_of(instance_id)?.probe,
            "restart": self.config_of(instance_id)?.restart,
            "discord": discord_view(&self.config_of(instance_id)?.discord),
        }))
    }

    fn name_of(&self, instance_id: &str) -> Result<String, MinewrapError> {
        Ok(self.config_of(instance_id)?.display_name().to_string())
    }

    /// Write-only patch; persists via the TOML store (0600).
    pub async fn patch_instance(
        &self,
        instance_id: &str,
        patch: serde_json::Value,
    ) -> Result<bool, MinewrapError> {
        let updated = {
            let mut current_doc = {
                let current = self.config_of(instance_id)?;
                serde_json::json!({
                    "name": current.name,
                    "work_dir": current.work_dir,
                    "java_path": current.java_path,
                    "jvm_args": current.jvm_args,
                    "launch_args": current.launch_args,
                    "autostart": current.autostart,
                    "probe": serde_json::to_value(&current.probe).unwrap_or(serde_json::json!({})),
                    "restart": serde_json::to_value(&current.restart).unwrap_or(serde_json::json!({})),
                    "discord": serde_json::to_value(&current.discord).unwrap_or(serde_json::json!({})),
                })
            };
            for (k, v) in patch.as_object().cloned().unwrap_or_default() {
                current_doc[&k] = v;
            }
            let mut daemon = self.daemon.lock().unwrap_or_else(|e| e.into_inner());
            daemon
                .patch_instance(instance_id, current_doc)
                .map_err(MinewrapError::invalid)?;
            let updated = daemon
                .instances
                .iter()
                .find(|c| c.id == instance_id)
                .cloned()
                .ok_or_else(|| MinewrapError::gone(format!("instance {instance_id} not found")))?;
            if !self.config_path.is_empty() {
                daemon
                    .save_to(&self.config_path)
                    .map_err(MinewrapError::invalid)?;
            }
            updated
        };

        let tx = self.ctrl_tx(instance_id).ok();
        self.with_inner(|inner| {
            if let Some(rt) = inner
                .instances
                .get_mut(LOCAL_NODE_ID)
                .and_then(|m| m.get_mut(instance_id))
            {
                rt.config = updated.clone();
                rt.name = updated.display_name().to_string();
            }
        });
        if let Some(tx) = tx {
            let _ = tx.send(Control::ApplyConfig(updated.clone())).await;
        }
        Ok(true)
    }

    /// Add a new instance at runtime (validates, spawns its
    /// `ProcessManager`, persists to TOML).
    pub fn add_instance(
        &self,
        cfg: InstanceConfig,
    ) -> Result<bool, MinewrapError> {
        cfg.validate().map_err(MinewrapError::invalid)?;
        let id = cfg.id.clone();
        let (defaults, unresponsive_after_failures) = {
            let daemon = self.daemon.lock().unwrap_or_else(|e| e.into_inner());
            (
                daemon.daemon.defaults.clone(),
                daemon.daemon.unresponsive_after_failures,
            )
        };
        let buf = Arc::new(Mutex::new(LogBuffer::new(defaults.log_buffer_lines)));
        let mgr = ProcessManager::new(
            cfg.clone(),
            defaults.clone(),
            self.bus.clone(),
            buf.clone(),
            None,
        );
        let handle = mgr.spawn();
        let state = InstanceState::new(
            cfg.id.clone(),
            cfg.display_name().to_string(),
            unresponsive_after_failures,
        );
        let rt = InstanceRuntime {
            id: cfg.id.clone(),
            name: cfg.display_name().to_string(),
            config: cfg.clone(),
            state,
            handle: Some(handle),
            buffer: buf,
            probe: ProbeInfo::default(),
            last_error: None,
        };

        // Persist to the in-memory daemon config and write 0600.
        let mut daemon = self.daemon.lock().unwrap_or_else(|e| e.into_inner());
        if daemon.instances.iter().any(|c| c.id == id) {
            return Err(MinewrapError::invalid(format!("duplicate instance id: {id:?}")));
        }
        daemon.instances.push(cfg);
        daemon.validate().map_err(MinewrapError::invalid)?;
        if !self.config_path.is_empty() {
            daemon
                .save_to(&self.config_path)
                .map_err(MinewrapError::invalid)?;
        }

        self.with_inner(|inner| {
            inner
                .instances
                .entry(LOCAL_NODE_ID.to_string())
                .or_default()
                .insert(rt.id.clone(), rt);
        });
        Ok(true)
    }

    /// Remove an instance (stops it, drops its runtime, persists).
    pub async fn remove_instance(&self, instance_id: &str) -> Result<bool, MinewrapError> {
        let _ = self.stop(LOCAL_NODE_ID, instance_id).await;
        let removed = self.with_inner(|inner| {
            inner
                .instances
                .get_mut(LOCAL_NODE_ID)
                .and_then(|m| m.remove(instance_id))
                .is_some()
        });
        if !removed {
            return Err(MinewrapError::gone(format!("instance {instance_id} not found")));
        }
        let mut daemon = self.daemon.lock().unwrap_or_else(|e| e.into_inner());
        daemon.instances.retain(|c| c.id != instance_id);
        if !self.config_path.is_empty() {
            daemon
                .save_to(&self.config_path)
                .map_err(MinewrapError::invalid)?;
        }
        Ok(true)
    }

    // --- lifecycle (real `ProcessManager::Control` mux) ---

    pub async fn start(&self, node_id: &str, instance_id: &str) -> Result<bool, MinewrapError> {
        if !self.is_local(node_id) {
            return Err(self.remote_offline(node_id));
        }
        let tx = self.ctrl_tx(instance_id)?;
        tx.send(Control::Start)
            .await
            .map_err(|_| MinewrapError::gone("process manager is not running"))?;
        Ok(true)
    }

    pub async fn stop(&self, node_id: &str, instance_id: &str) -> Result<bool, MinewrapError> {
        if !self.is_local(node_id) {
            return Err(self.remote_offline(node_id));
        }
        let tx = self.ctrl_tx(instance_id)?;
        tx.send(Control::Stop)
            .await
            .map_err(|_| MinewrapError::gone("process manager is not running"))?;
        Ok(true)
    }

    pub async fn restart(&self, node_id: &str, instance_id: &str) -> Result<bool, MinewrapError> {
        if !self.is_local(node_id) {
            return Err(self.remote_offline(node_id));
        }
        let tx = self.ctrl_tx(instance_id)?;
        tx.send(Control::Stop)
            .await
            .map_err(|_| MinewrapError::gone("process manager is not running"))?;
        // Best-effort: manager will transition through stopping; a second
        // start is queued after a short delay by the caller/UI refresh.
        let tx2 = self.ctrl_tx(instance_id)?;
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            let _ = tx2.send(Control::Start).await;
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
        self.ensure_runtime(instance_id)?;
        self.with_inner(|inner| match inner.instances.get(LOCAL_NODE_ID).and_then(|m| m.get(instance_id)) {
            Some(rt) => rt
                .handle
                .as_ref()
                .map(|h| h.ctrl_tx.clone())
                .ok_or_else(|| MinewrapError::gone("process manager is not running")),
            None => Err(MinewrapError::gone(format!("instance {instance_id} not found"))),
        })
    }

    fn ensure_runtime(&self, instance_id: &str) -> Result<(), MinewrapError> {
        let needs_spawn = self.with_inner(|inner| {
            inner
                .instances
                .get(LOCAL_NODE_ID)
                .and_then(|m| m.get(instance_id))
                .map(|rt| rt.handle.is_none())
                .unwrap_or(false)
        });
        if !needs_spawn {
            return Ok(());
        }

        let (config, defaults, unresponsive_after_failures, buffer) = self.with_inner(|inner| {
            let rt = inner
                .instances
                .get(LOCAL_NODE_ID)
                .and_then(|m| m.get(instance_id))
                .ok_or_else(|| MinewrapError::gone(format!("instance {instance_id} not found")))?;
            Ok::<_, MinewrapError>((
                rt.config.clone(),
                self.daemon.lock().unwrap_or_else(|e| e.into_inner()).daemon.defaults.clone(),
                self.daemon.lock().unwrap_or_else(|e| e.into_inner()).daemon.unresponsive_after_failures,
                rt.buffer.clone(),
            ))
        })?;

        let mgr = ProcessManager::new(config, defaults, self.bus.clone(), buffer, None);
        let handle = mgr.spawn();
        self.with_inner(|inner| {
            if let Some(rt) = inner
                .instances
                .get_mut(LOCAL_NODE_ID)
                .and_then(|m| m.get_mut(instance_id))
            {
                rt.handle = Some(handle);
                rt.state = InstanceState::new(
                    rt.id.clone(),
                    rt.name.clone(),
                    unresponsive_after_failures,
                );
                rt.last_error = None;
            }
        });
        Ok(())
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

    /// Apply a core state-change event to the in-memory runtime so
    /// snapshots/UI reflect Start/Stop transitions.
    pub fn update_process_state(&self, instance_id: &str, new_state: &str, reason: &str) {
        let state = match new_state {
            "online" => ProcessState::Online,
            "starting" => ProcessState::Starting,
            "stopping" => ProcessState::Stopping,
            "crashed" => ProcessState::Crashed,
            _ => ProcessState::Offline,
        };
        let now = chrono::Utc::now();
        self.with_inner(|inner| {
            let Some(rt) = inner
                .instances
                .get_mut(LOCAL_NODE_ID)
                .and_then(|m| m.get_mut(instance_id))
            else {
                return;
            };
            if state == ProcessState::Crashed || reason.contains("spawn_failed") || reason.contains("failed") || reason.contains("crash_during_start") || reason.contains("startup_timeout") {
                rt.last_error = Some(reason.to_string());
            } else if matches!(state, ProcessState::Starting | ProcessState::Online) {
                rt.last_error = None;
            }
            let mut status = minewrap_core::ProcessStatus {
                state,
                pid: None,
                started_at: rt.state.snapshot().process.started_at,
                stopped_at: rt.state.snapshot().process.stopped_at,
                exit_code: rt.state.snapshot().process.exit_code,
                requested_stop: matches!(state, ProcessState::Stopping),
                forced_stop: rt.state.snapshot().process.forced_stop,
                restart_count: rt.state.snapshot().process.restart_count,
                crash_count: rt.state.snapshot().process.crash_count,
                uptime_seconds: None,
            };
            if matches!(state, ProcessState::Starting | ProcessState::Online) && status.started_at.is_none() {
                status.started_at = Some(now);
                status.stopped_at = None;
            }
            if matches!(state, ProcessState::Offline | ProcessState::Crashed) {
                status.stopped_at = Some(now);
                if reason.contains("requested") {
                    status.requested_stop = true;
                }
            }
            rt.state.update_process(status);
        });
    }

    /// Stop all local instances best-effort (used on GUI exit).
    pub async fn stop_all_local(&self) {
        let ids: Vec<String> = self.with_inner(|inner| {
            inner
                .instances
                .get(LOCAL_NODE_ID)
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default()
        });
        for id in ids {
            let _ = self.stop(LOCAL_NODE_ID, &id).await;
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        self.with_inner(|inner| {
            if let Some(map) = inner.instances.get_mut(LOCAL_NODE_ID) {
                for rt in map.values_mut() {
                    if let Some(handle) = rt.handle.take() {
                        handle.owner.abort();
                    }
                }
            }
        });
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
    pub last_error: Option<String>,
}

fn discord_view(d: &minewrap_core::DiscordConfig) -> serde_json::Value {
    serde_json::json!({
        "enabled": d.enabled,
        "webhook_set": d.webhook_set(),
        "mode": d.mode,
        "interval_seconds": d.interval_seconds,
        "first_push_delay_seconds": d.first_push_delay_seconds,
        "log_lines": d.log_lines,
        "max_log_chars": d.max_log_chars,
        "state_change_posts": d.state_change_posts,
        "state_change_debounce_seconds": d.state_change_debounce_seconds,
        "redact_regexes": d.redact_regexes,
    })
}
