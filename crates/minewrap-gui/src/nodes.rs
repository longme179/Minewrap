//! Node registry + per-node client surface for the Slint GUI.
//!
//! Port of `src-tauri/src/node_service.rs` (P0 daemon-first): pure Rust,
//! no Tauri. Remote nodes are persisted in `nodes.json` (mode 0600) under
//! the XDG config dir; the local node is the `minewrapd` this GUI
//! auto-spawns (URL + token from the daemon spawner in `daemon.rs`).
//!
//! Tokens live only in Rust (nodes.json / `{data_dir}/token`) — they are
//! never logged and never echoed back by the UI after entry.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use minewrap_remote::types::{
    Action, DiscordTestResponse, InstanceSummaryDto, JavaRuntimeDto, LocalSettings, LogLineDto,
    MinewrapError, ModDto, NodeState, NodeSummary, PropertiesDoc, SnapshotDto, load_local_settings,
};
pub const LOCAL_NODE_ID: &str = "local";

/// A persisted remote-node record (`nodes.json` entry).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NodeRecord {
    pub id: String,
    pub name: String,
    pub url: String,
    pub token: String,
}

struct NodeEntry {
    summary: NodeSummary,
    record: Option<NodeRecord>, // None for the local node
    client: Option<minewrap_remote::RemoteClient>,
}

struct Inner {
    nodes: HashMap<String, NodeEntry>,
}

/// All nodes known to the GUI: the local node (auto) + persisted remotes.
pub struct NodeService {
    inner: Mutex<Inner>,
    /// The `minewrap.toml` the GUI loaded (host/port/data_dir source).
    pub config_path: String,
    pub local: LocalSettings,
    /// Where `nodes.json` lives — resolved ONCE at load so the process
    /// env is never re-read (and tests can point it at a scratch path
    /// without mutating the global `XDG_CONFIG_HOME`, which races under
    /// the default parallel test runner).
    nodes_path: PathBuf,
}

/// One node + its instances, as shipped from the runtime poll to the UI.
#[derive(Clone)]
pub struct NodeInstances {
    pub node: NodeSummary,
    pub instances: Vec<InstanceSummaryDto>,
}

fn nodes_json_path() -> PathBuf {
    let dir = if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(x).join("minewrap")
    } else if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".config").join("minewrap")
    } else {
        PathBuf::from(".")
    };
    std::fs::create_dir_all(&dir).ok();
    dir.join("nodes.json")
}

impl NodeService {
    /// Load settings + the node registry. The local node starts as
    /// `Connecting`; `main.rs` spawns the daemon and the health poll
    /// flips the state from `/health` probes.
    pub fn load(config_path: &str) -> Result<Self, MinewrapError> {
        let nodes_path = nodes_json_path();
        Self::load_at(config_path, nodes_path)
    }

    /// Same as [`load`] but with an explicit registry path (test seam:
    /// keeps tests off the process-global XDG env).
    fn load_at(config_path: &str, nodes_path: PathBuf) -> Result<Self, MinewrapError> {
        let local = load_local_settings(config_path)
            .map_err(|e| MinewrapError::invalid(format!("config: {e}")))?;
        std::fs::create_dir_all(&nodes_path.parent().unwrap_or(Path::new("."))).ok();
        let svc = Self {
            inner: Mutex::new(Inner {
                nodes: HashMap::new(),
            }),
            config_path: config_path.to_string(),
            local,
            nodes_path,
        };
        svc.upsert_local(NodeState::Connecting);
        for record in Self::load_records(&svc.nodes_path) {
            svc.upsert_record(record, NodeState::Connecting);
        }
        Ok(svc)
    }

    fn load_records(path: &Path) -> Vec<NodeRecord> {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Vec::new();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    fn save_records(path: &Path, records: &[NodeRecord]) {
        let data = serde_json::to_string_pretty(records).unwrap_or_else(|_| "[]".into());
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, &data).is_ok() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(
                    &tmp,
                    std::fs::Permissions::from_mode(0o600),
                );
            }
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    fn records_locked(inner: &Inner) -> Vec<NodeRecord> {
        inner
            .nodes
            .values()
            .filter_map(|e| e.record.clone())
            .collect()
    }

    fn upsert_local(&self, state: NodeState) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.nodes.insert(
            LOCAL_NODE_ID.to_string(),
            NodeEntry {
                summary: NodeSummary {
                    id: LOCAL_NODE_ID.to_string(),
                    name: "Local".to_string(),
                    url: "local".to_string(),
                    is_local: true,
                    state,
                },
                record: None,
                client: None,
            },
        );
    }

    fn upsert_record(&self, record: NodeRecord, state: NodeState) {
        let client = minewrap_remote::RemoteClient::new(record.url.clone(), record.token.clone());
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.nodes.insert(
            record.id.clone(),
            NodeEntry {
                summary: NodeSummary {
                    id: record.id.clone(),
                    name: record.name.clone(),
                    url: record.url.clone(),
                    is_local: false,
                    state,
                },
                record: Some(record),
                client: Some(client),
            },
        );
    }

    /// The local daemon's base URL.
    pub fn local_url(&self) -> String {
        format!("http://{}:{}", self.local.host, self.local.port)
    }

    /// Read the local token (`{data_dir}/token`, written by minewrapd).
    pub fn local_token(&self) -> String {
        std::fs::read_to_string(PathBuf::from(&self.local.data_dir).join("token"))
            .map(|t| t.trim().to_string())
            .unwrap_or_default()
    }

    /// Build (or rebuild) the local client. The built client is cached;
    /// pass `refresh = true` to re-read the token (e.g. after a 401).
    fn local_client(&self, refresh: bool) -> minewrap_remote::RemoteClient {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let entry = inner.nodes.entry(LOCAL_NODE_ID.to_string()).or_insert_with(|| NodeEntry {
            summary: NodeSummary {
                id: LOCAL_NODE_ID.to_string(),
                name: "Local".to_string(),
                url: "local".to_string(),
                is_local: true,
                state: NodeState::Offline,
            },
            record: None,
            client: None,
        });
        if refresh || entry.client.is_none() {
            entry.client = Some(minewrap_remote::RemoteClient::new(
                self.local_url(),
                self.local_token(),
            ));
        }
        entry.client.clone().expect("just set")
    }

    /// The client for a node (cloned — cheap, `reqwest::Client` is
    /// refcounted). `refresh` re-reads the local token.
    pub fn client_of(
        &self,
        node_id: &str,
        refresh: bool,
    ) -> Result<minewrap_remote::RemoteClient, MinewrapError> {
        if node_id == LOCAL_NODE_ID {
            return Ok(self.local_client(refresh));
        }
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match inner.nodes.get(node_id) {
            Some(entry) => entry
                .client
                .clone()
                .ok_or_else(|| MinewrapError::offline(format!("node {node_id}: no client"))),
            None => Err(MinewrapError::gone(format!("node {node_id} not found"))),
        }
    }

    fn with_retry_401(&self, node_id: &str) -> Result<minewrap_remote::RemoteClient, MinewrapError> {
        self.client_of(node_id, false)
    }

    /// Set a node's connection state (health probes).
    pub fn set_node_state(&self, node_id: &str, state: NodeState) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = inner.nodes.get_mut(node_id) {
            entry.summary.state = state;
        }
    }

    // --- node registry ---

    pub fn list_nodes(&self) -> Result<Vec<NodeSummary>, MinewrapError> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut nodes: Vec<NodeSummary> =
            inner.nodes.values().map(|e| e.summary.clone()).collect();
        nodes.sort_by(|a, b| b.is_local.cmp(&a.is_local).then(a.id.cmp(&b.id)));
        Ok(nodes)
    }

    pub fn add_remote_node(
        &self,
        name: String,
        url: String,
        token: String,
    ) -> Result<NodeSummary, MinewrapError> {
        let id = format!("remote-{}", url.replace(['/', ':', '.'], "-"));
        let record = NodeRecord {
            id: id.clone(),
            name,
            url,
            token,
        };
        self.upsert_record(record, NodeState::Connecting);
        let records = {
            let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            Self::records_locked(&inner)
        };
        Self::save_records(&self.nodes_path, &records);
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .nodes
            .get(&id)
            .map(|e| e.summary.clone())
            .ok_or_else(|| MinewrapError::gone(format!("node {id} vanished")))
    }

    pub fn remove_remote_node(&self, id: &str) -> Result<bool, MinewrapError> {
        let removed = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            match inner.nodes.get(id) {
                Some(e) if e.summary.is_local => {
                    return Err(MinewrapError::invalid("cannot remove the local node"));
                }
                Some(_) => inner.nodes.remove(id).is_some(),
                None => false,
            }
        };
        if removed {
            let records = {
                let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                Self::records_locked(&inner)
            };
            Self::save_records(&self.nodes_path, &records);
        }
        Ok(removed)
    }

    /// Health-probe one node and update its cached state.
    pub async fn probe_node(&self, node_id: &str) -> Result<NodeState, MinewrapError> {
        let client = self.with_retry_401(node_id)?;
        match client.health_timeout(std::time::Duration::from_secs(3)).await {
            Ok(_) => {
                self.set_node_state(node_id, NodeState::Connected);
                Ok(NodeState::Connected)
            }
            Err(e) => {
                let state = minewrap_remote::RemoteClient::state_from_error(&e);
                self.set_node_state(node_id, state);
                Err(e)
            }
        }
    }

    // --- instance surface (all via REST, routed to the node's client) ---

    pub async fn list_instances(&self, node_id: &str) -> Result<Vec<InstanceSummaryDto>, MinewrapError> {
        let client = self.with_retry_401(node_id)?;
        match client.list_instances_typed().await {
            Ok(v) => Ok(v),
            Err(e) if e.code == "unauthorised" && node_id == LOCAL_NODE_ID => {
                let client = self.client_of(node_id, true)?;
                client.list_instances_typed().await
            }
            Err(e) => Err(e),
        }
    }

    /// An unknown instance yields the offline placeholder snapshot
    /// instead of an error (v1.9.3 GUI parity).
    pub async fn snapshot(&self, node_id: &str, instance_id: &str) -> Result<SnapshotDto, MinewrapError> {
        let client = self.with_retry_401(node_id)?;
        match client.snapshot_typed(instance_id).await {
            Ok(s) => Ok(s),
            Err(e) if e.code == "gone" => Ok(SnapshotDto::offline_placeholder(instance_id)),
            Err(e) if e.code == "unauthorised" && node_id == LOCAL_NODE_ID => {
                let client = self.client_of(node_id, true)?;
                match client.snapshot_typed(instance_id).await {
                    Ok(s) => Ok(s),
                    Err(e2) if e2.code == "gone" => Ok(SnapshotDto::offline_placeholder(instance_id)),
                    Err(e2) => Err(e2),
                }
            }
            Err(e) => Err(e),
        }
    }

    async fn action_call(
        &self,
        node_id: &str,
        instance_id: &str,
        action: &str,
    ) -> Result<Action, MinewrapError> {
        let client = self.with_retry_401(node_id)?;
        let v = client.action(instance_id, action, None).await?;
        Ok(action_from(v))
    }

    pub async fn start(&self, node_id: &str, instance_id: &str) -> Result<Action, MinewrapError> {
        self.action_call(node_id, instance_id, "start").await
    }

    pub async fn stop(&self, node_id: &str, instance_id: &str) -> Result<Action, MinewrapError> {
        self.action_call(node_id, instance_id, "stop").await
    }

    pub async fn restart(&self, node_id: &str, instance_id: &str) -> Result<Action, MinewrapError> {
        self.action_call(node_id, instance_id, "restart").await
    }

    pub async fn send_command(
        &self,
        node_id: &str,
        instance_id: &str,
        command: &str,
    ) -> Result<Action, MinewrapError> {
        let cmd = command.trim().to_string();
        if cmd.is_empty() {
            return Err(MinewrapError::invalid("command must not be empty"));
        }
        if cmd.contains('\n') || cmd.contains('\r') {
            return Err(MinewrapError::invalid("command must be a single line"));
        }
        let client = self.with_retry_401(node_id)?;
        let v = client.send_command(instance_id, &cmd).await?;
        Ok(action_from(v))
    }

    pub async fn get_logs(
        &self,
        node_id: &str,
        instance_id: &str,
        limit: usize,
        since_seq: u64,
    ) -> Result<Vec<LogLineDto>, MinewrapError> {
        let client = self.with_retry_401(node_id)?;
        client.logs_typed(instance_id, limit, since_seq).await
    }

    pub async fn get_config(
        &self,
        node_id: &str,
        instance_id: &str,
    ) -> Result<serde_json::Value, MinewrapError> {
        let client = self.with_retry_401(node_id)?;
        client.get_config(instance_id).await
    }

    pub async fn update_config(
        &self,
        node_id: &str,
        instance_id: &str,
        patch: serde_json::Value,
    ) -> Result<Action, MinewrapError> {
        let client = self.with_retry_401(node_id)?;
        client.update_config(instance_id, patch).await
    }

    /// `add_instance` is local-node only (instance config lives on the
    /// daemon host).
    pub async fn add_instance(&self, config: serde_json::Value) -> Result<Action, MinewrapError> {
        let client = self.with_retry_401(LOCAL_NODE_ID)?;
        client.add_instance(config).await
    }

    pub async fn remove_instance(
        &self,
        node_id: &str,
        instance_id: &str,
    ) -> Result<Action, MinewrapError> {
        let client = self.with_retry_401(node_id)?;
        client.remove_instance(instance_id).await
    }

    /// `detect_java` is local-node only (runs on the daemon host).
    pub async fn detect_java(&self) -> Result<Vec<JavaRuntimeDto>, MinewrapError> {
        let client = self.with_retry_401(LOCAL_NODE_ID)?;
        client.detect_java().await
    }

    pub async fn scan_launch(
        &self,
        node_id: &str,
        work_dir: &str,
    ) -> Result<serde_json::Value, MinewrapError> {
        let client = self.with_retry_401(node_id)?;
        client.scan_workdir(work_dir).await
    }

    pub async fn read_properties(
        &self,
        node_id: &str,
        instance_id: &str,
    ) -> Result<PropertiesDoc, MinewrapError> {
        let client = self.with_retry_401(node_id)?;
        client.read_properties(instance_id).await
    }

    pub async fn write_properties(
        &self,
        node_id: &str,
        instance_id: &str,
        text: &str,
    ) -> Result<Action, MinewrapError> {
        let client = self.with_retry_401(node_id)?;
        client.write_properties(instance_id, text).await
    }

    pub async fn list_mods(
        &self,
        node_id: &str,
        instance_id: &str,
    ) -> Result<Vec<ModDto>, MinewrapError> {
        let client = self.with_retry_401(node_id)?;
        client.list_mods(instance_id).await
    }

    pub async fn toggle_mod(
        &self,
        node_id: &str,
        instance_id: &str,
        name: &str,
    ) -> Result<Action, MinewrapError> {
        let client = self.with_retry_401(node_id)?;
        client.toggle_mod(instance_id, name).await
    }

    pub async fn discord_test(
        &self,
        node_id: &str,
        instance_id: &str,
    ) -> Result<DiscordTestResponse, MinewrapError> {
        let client = self.with_retry_401(node_id)?;
        client.discord_test(instance_id).await
    }

    pub async fn health(&self, node_id: &str) -> Result<Action, MinewrapError> {
        let client = self.with_retry_401(node_id)?;
        let h = client.health().await?;
        Ok(Action {
            ok: true,
            detail: Some(h.api_version),
        })
    }
}

/// The daemon's action endpoints return `{"ok": bool}` or
/// `{"accepted": bool}` depending on the route (v1 shapes).
fn action_from(v: serde_json::Value) -> Action {
    let ok = v
        .get("ok")
        .and_then(|x| x.as_bool())
        .or_else(|| v.get("accepted").and_then(|x| x.as_bool()))
        .unwrap_or(true);
    let detail = v.get("detail").and_then(|x| x.as_str()).map(String::from);
    Action { ok, detail }
}

/// Map a node `NodeState` to a UI label + dot code (reuses
/// `instances.rs` codes: 0 online … 6 unknown).
pub fn node_state_summary(state: NodeState) -> (String, i32) {
    match state {
        NodeState::Connected => ("online".to_string(), 0),
        NodeState::Connecting => ("connecting".to_string(), 1),
        NodeState::Offline => ("offline".to_string(), 3),
        NodeState::Unauthorised => ("unauthorised".to_string(), 4),
        NodeState::Incompatible => ("incompatible".to_string(), 4),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(dir: &std::path::Path) -> String {
        let path = dir.join("minewrap.toml");
        std::fs::write(
            &path,
            "[daemon]\nhost = \"127.0.0.1\"\nport = 8390\ndata_dir = \"./data\"\n",
        )
        .unwrap();
        path.to_string_lossy().to_string()
    }

    #[test]
    fn records_round_trip_keeps_0600() {
        // Own scratch dir + explicit registry path: NEVER touch the
        // process-global XDG env (parallel tests race on set_var — that
        // race is exactly what this suite's flake was).
        let dir = std::env::temp_dir().join(format!(
            "minewrap-nodes-test-roundtrip-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let registry = dir.join("minewrap").join("nodes.json");
        let _config = test_config(&dir);

        let svc = NodeService::load_at(&test_config(&dir), registry.clone()).unwrap();
        let summary = svc
            .add_remote_node(
                "Home".to_string(),
                "http://192.168.1.10:8080".to_string(),
                "secret-token".to_string(),
            )
            .unwrap();
        assert_eq!(summary.name, "Home");
        assert!(!summary.is_local);

        // Persisted file parses back and is 0600.
        let path = registry.clone();
        let text = std::fs::read_to_string(&path).unwrap();
        let records: Vec<NodeRecord> = serde_json::from_str(&text).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].token, "secret-token");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "nodes.json must be 0600");
        }

        // Local node sorts first; remote added after.
        let nodes = svc.list_nodes().unwrap();
        assert_eq!(nodes[0].id, LOCAL_NODE_ID);
        assert_eq!(nodes[0].name, "Local");
        assert!(nodes[0].is_local);
        assert_eq!(nodes[1].id, summary.id);

        // Removing the remote node empties the registry file.
        assert!(svc.remove_remote_node(&summary.id).unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        let records: Vec<NodeRecord> = serde_json::from_str(&text).unwrap();
        assert_eq!(records.len(), 0);
        assert!(!svc.remove_remote_node(&summary.id).unwrap()); // already gone

        // The local node cannot be removed.
        assert!(svc.remove_remote_node(LOCAL_NODE_ID).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn client_of_remote_and_local() {
        let dir = std::env::temp_dir().join(format!(
            "minewrap-nodes-test-client-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config = test_config(&dir);
        let registry = dir.join("minewrap").join("nodes.json");
        let svc = NodeService::load_at(&config, registry).unwrap();

        let summary = svc
            .add_remote_node("R".to_string(), "http://127.0.0.1:8391".to_string(), "tok".to_string())
            .unwrap();
        let client = svc.client_of(&summary.id, false).unwrap();
        assert_eq!(client.base(), "http://127.0.0.1:8391");

        let local = svc.client_of(LOCAL_NODE_ID, false).unwrap();
        assert_eq!(local.base(), svc.local_url());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn state_from_error_maps_codes() {
        assert_eq!(
            minewrap_remote::RemoteClient::state_from_error(&MinewrapError::unauthorised("x")),
            NodeState::Unauthorised
        );
        assert_eq!(
            minewrap_remote::RemoteClient::state_from_error(&MinewrapError::incompatible("x")),
            NodeState::Incompatible
        );
        assert_eq!(
            minewrap_remote::RemoteClient::state_from_error(&MinewrapError::offline("x")),
            NodeState::Offline
        );
    }

    #[test]
    fn node_state_summary_labels() {
        assert_eq!(node_state_summary(NodeState::Connected), ("online".to_string(), 0));
        assert_eq!(node_state_summary(NodeState::Connecting), ("connecting".to_string(), 1));
        assert_eq!(node_state_summary(NodeState::Offline), ("offline".to_string(), 3));
        assert_eq!(node_state_summary(NodeState::Unauthorised), ("unauthorised".to_string(), 4));
        assert_eq!(
            node_state_summary(NodeState::Incompatible),
            ("incompatible".to_string(), 4)
        );
    }
}
