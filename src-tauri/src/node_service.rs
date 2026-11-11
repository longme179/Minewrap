//! `NodeService` v1.10.0 (P0 daemon-first): the GUI is a **pure
//! client**. Every IPC call routes through `minewrap-remote` to a
//! `minewrapd` — the local node is just the daemon on
//! `127.0.0.1:{port}` (auto-spawned by the shell, see `main.rs`), and
//! remote nodes live in the registry file `nodes.json` (0600). There
//! is no embedded `minewrap-core` anymore: closing the GUI only closes
//! the window, servers keep running in the daemon.
//!
//! Token handling: the local token is read from
//! `{data_dir}/token` (written by the daemon); remote tokens live in
//! `nodes.json`. Tokens never enter the webview.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use minewrap_remote::types::{
    Action, DiscordTestResponse, InstanceSummaryDto, JavaRuntimeDto, LocalSettings, LogLineDto,
    MinewrapError, ModDto, NodeState, NodeSummary, PropertiesDoc, SnapshotDto, load_local_settings,
};

pub const LOCAL_NODE_ID: &str = "local";

/// A persisted remote-node record (nodes.json entry).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct NodeRecord {
    id: String,
    name: String,
    url: String,
    token: String,
}

struct NodeEntry {
    summary: NodeSummary,
    record: Option<NodeRecord>, // None for the local node
    client: Option<minewrap_remote::RemoteClient>,
}

struct Inner {
    nodes: HashMap<String, NodeEntry>,
}

pub struct NodeService {
    inner: Mutex<Inner>,
    /// The minewrap.toml the GUI loaded (host/port/data_dir source).
    pub config_path: String,
    pub local: LocalSettings,
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
    /// `connecting`; `main.rs` spawns the daemon and the forwarder
    /// flips the state from the `/events` connection.
    pub fn load(config_path: &str) -> Result<Self, MinewrapError> {
        let local = load_local_settings(config_path)
            .map_err(|e| MinewrapError::invalid(format!("config: {e}")))?;
        let svc = Self {
            inner: Mutex::new(Inner {
                nodes: HashMap::new(),
            }),
            config_path: config_path.to_string(),
            local,
        };
        svc.upsert_local(NodeState::Connecting);
        for record in Self::load_records() {
            svc.upsert_record(record, NodeState::Connecting);
        }
        Ok(svc)
    }

    fn load_records() -> Vec<NodeRecord> {
        let path = nodes_json_path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Vec::new();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    fn save_records(records: &[NodeRecord]) {
        let path = nodes_json_path();
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
                    name: "This machine".to_string(),
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

    /// Build (or rebuild) the local client. Rebuilt on every call is
    /// wasteful, so the built client is cached; pass `refresh = true`
    /// to re-read the token (e.g. after a 401).
    fn local_client(&self, refresh: bool) -> minewrap_remote::RemoteClient {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let entry = inner.nodes.entry(LOCAL_NODE_ID.to_string()).or_insert_with(|| NodeEntry {
            summary: NodeSummary {
                id: LOCAL_NODE_ID.to_string(),
                name: "This machine".to_string(),
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
    pub fn client_of(&self, node_id: &str, refresh: bool) -> Result<minewrap_remote::RemoteClient, MinewrapError> {
        if node_id == LOCAL_NODE_ID {
            return Ok(self.local_client(refresh));
        }
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match inner.nodes.get(node_id) {
            Some(entry) => entry.client.clone().ok_or_else(|| {
                MinewrapError::offline(format!("node {node_id}: no client"))
            }),
            None => Err(MinewrapError::gone(format!("node {node_id} not found"))),
        }
    }

    fn with_retry_401(
        &self,
        node_id: &str,
    ) -> Result<minewrap_remote::RemoteClient, MinewrapError> {
        self.client_of(node_id, false)
    }

    /// Set a node's connection state (forwarder / health probes).
    pub fn set_node_state(&self, node_id: &str, state: NodeState) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = inner.nodes.get_mut(node_id) {
            entry.summary.state = state;
        }
    }

    // --- node registry ---

    pub fn list_nodes(&self) -> Result<Vec<NodeSummary>, MinewrapError> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut nodes: Vec<NodeSummary> = inner.nodes.values().map(|e| e.summary.clone()).collect();
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
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let records = Self::records_locked(&inner);
        drop(inner);
        Self::save_records(&records);
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
            Self::save_records(&records);
        }
        Ok(removed)
    }

    /// Health-probe one node and update its state.
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

    // --- instance surface (all via REST) ---

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

    /// v1.9.3 GUI parity: an unknown instance yields the offline
    /// placeholder snapshot instead of an error.
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

    pub async fn add_instance(&self, config: serde_json::Value) -> Result<Action, MinewrapError> {
        let client = self.with_retry_401(LOCAL_NODE_ID)?;
        client.add_instance(config).await
    }

    pub async fn remove_instance(&self, node_id: &str, instance_id: &str) -> Result<Action, MinewrapError> {
        let client = self.with_retry_401(node_id)?;
        client.remove_instance(instance_id).await
    }

    pub async fn detect_java(&self) -> Result<Vec<JavaRuntimeDto>, MinewrapError> {
        let client = self.with_retry_401(LOCAL_NODE_ID)?;
        client.detect_java().await
    }

    pub async fn scan_launch(&self, node_id: &str, work_dir: &str) -> Result<serde_json::Value, MinewrapError> {
        let client = self.with_retry_401(node_id)?;
        client.scan_workdir(work_dir).await
    }

    pub async fn read_properties(&self, node_id: &str, instance_id: &str) -> Result<PropertiesDoc, MinewrapError> {
        let client = self.with_retry_401(node_id)?;
        client.read_properties(instance_id).await
    }

    pub async fn write_properties(&self, node_id: &str, instance_id: &str, text: &str) -> Result<Action, MinewrapError> {
        let client = self.with_retry_401(node_id)?;
        client.write_properties(instance_id, text).await
    }

    pub async fn list_mods(&self, node_id: &str, instance_id: &str) -> Result<Vec<ModDto>, MinewrapError> {
        let client = self.with_retry_401(node_id)?;
        client.list_mods(instance_id).await
    }

    pub async fn toggle_mod(&self, node_id: &str, instance_id: &str, name: &str) -> Result<Action, MinewrapError> {
        let client = self.with_retry_401(node_id)?;
        client.toggle_mod(instance_id, name).await
    }

    pub async fn discord_test(&self, node_id: &str, instance_id: &str) -> Result<DiscordTestResponse, MinewrapError> {
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

    // --- local daemon auto-spawn (task B7) ---

    /// Locate the `minewrapd` binary: next to the exe (AppImage
    /// `usr/bin`, portable `binaries/`) then `PATH`.
    fn find_daemon_binary() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let dir = exe.parent()?;
        let mut candidates = vec![
            dir.join("minewrapd"),
            dir.join("binaries").join("minewrapd"),
            dir.join("binaries").join("minewrapd-x86_64-unknown-linux-gnu"),
        ];
        if let Ok(path_var) = std::env::var("PATH") {
            for p in path_var.split(':') {
                if !p.is_empty() {
                    candidates.push(PathBuf::from(p).join("minewrapd"));
                }
            }
        }
        candidates.into_iter().find(|c| c.exists())
    }

    /// One `/health` probe from synchronous context.
    fn health_probe(&self) -> bool {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("probe runtime");
        rt.block_on(async {
            let client = minewrap_remote::RemoteClient::new(self.local_url(), self.local_token());
            client
                .health_timeout(std::time::Duration::from_secs(1))
                .await
                .is_ok()
        })
    }

    /// Probe `GET /health`; on refusal spawn the daemon **detached**
    /// (own process group, stdio appended to `{data_dir}/daemon.log`)
    /// with the config this GUI loaded, then wait up to 10 s for
    /// health. Also the retry path for the UI when the daemon died.
    pub fn ensure_local_daemon(&self) -> Result<(), String> {
        if self.health_probe() {
            return Ok(());
        }
        let binary = Self::find_daemon_binary()
            .ok_or_else(|| "minewrapd not found next to the app or on PATH".to_string())?;
        let data_dir = PathBuf::from(&self.local.data_dir);
        std::fs::create_dir_all(&data_dir).map_err(|e| format!("data dir: {e}"))?;
        let log_path = data_dir.join("daemon.log");
        let stdout = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|e| format!("open {}: {e}", log_path.display()))?;
        let stderr = stdout
            .try_clone()
            .map_err(|e| format!("clone log handle: {e}"))?;
        let mut command = std::process::Command::new(&binary);
        command
            .env("MINEWRAP_CONFIG", &self.config_path)
            .stdin(std::process::Stdio::null())
            .stdout(stdout)
            .stderr(stderr);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0); // setsid: survive the GUI
        }
        command
            .spawn()
            .map_err(|e| format!("spawn {}: {e}", binary.display()))?;
        eprintln!(
            "minewrap: spawned daemon {} (config {}, log {})",
            binary.display(),
            self.config_path,
            log_path.display()
        );
        // Wait <= 10 s for /health (250 ms polls); the fresh daemon may
        // have (re)generated the token, so re-read it each poll.
        for _ in 0..40 {
            if self.health_probe() {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        Err("daemon did not become healthy within 10 s".to_string())
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
    let detail = v
        .get("detail")
        .and_then(|x| x.as_str())
        .map(String::from);
    Action { ok, detail }

}
