//! `NodeService`: routes IPC calls by `node_id`.
//!
//! The local branch talks to the embedded `minewrap-core`. The remote
//! branch is a placeholder that reports `offline` until Phase 5 wires the
//! `minewrap-remote` (reqwest) client — the frontend never branches on
//! local/remote, and tokens never enter the webview.

use std::collections::HashMap;
use std::sync::Arc;

use minewrap_core::{
    InstanceState, InstanceSummary, LogLine, ProcessManager, ProcessState, ProbeInfo,
};

use crate::types::{LOCAL_NODE_ID, MinewrapError, NodeState, NodeSummary};

/// One managed instance: its process manager + merged state.
pub struct InstanceRuntime {
    pub id: String,
    pub name: String,
    pub state: InstanceState,
    /// Kept so `drop` of the `OwnerHandle` is explicit in Phase 5's
    /// shutdown path; held as `Option` because the owner task owns the
    /// manager.
    #[allow(dead_code)]
    pub handle: Option<ProcessManager>,
}

#[derive(Default)]
pub struct NodeServiceInner {
    pub nodes: HashMap<String, NodeSummary>,
    pub instances: HashMap<String, HashMap<String, InstanceRuntime>>,
}

pub struct NodeService {
    pub inner: std::sync::Mutex<NodeServiceInner>,
}

impl NodeService {
    pub fn new() -> Self {
        let mut service = Self {
            inner: std::sync::Mutex::new(NodeServiceInner::default()),
        };

        let mut guard = match service.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        guard.nodes.insert(
            LOCAL_NODE_ID.to_string(),
            NodeSummary {
                id: LOCAL_NODE_ID.to_string(),
                name: "This machine".to_string(),
                url: "local".to_string(),
                is_local: true,
                state: NodeState::Connected,
            },
        );
        service
    }

    pub fn is_local(&self, node_id: &str) -> bool {
        node_id == LOCAL_NODE_ID
    }

    pub fn list_nodes(&self) -> Result<Vec<NodeSummary>, MinewrapError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| MinewrapError::offline("node service locked"))?;
        Ok(guard.nodes.values().cloned().collect())
    }

    pub fn list_instances(&self, node_id: &str) -> Result<Vec<InstanceSummary>, MinewrapError> {
        if !self.is_local(node_id) {
            return Err(self.remote_offline(node_id));
        }
        let guard = self
            .inner
            .lock()
            .map_err(|_| MinewrapError::offline("node service locked"))?;
        match guard.instances.get(node_id) {
            Some(map) => Ok(map.values().map(|rt| rt.state.summary()).collect()),
            None => Ok(Vec::new()),
        }
    }

    fn remote_offline(&self, node_id: &str) -> MinewrapError {
        MinewrapError::offline(format!(
            "remote node {node_id} is not reachable (Phase 5: minewrap-remote client)"
        ))
    }

    pub fn snapshot(&self, node_id: &str, instance_id: &str) -> Result<SnapshotDto, MinewrapError> {
        if !self.is_local(node_id) {
            return Err(self.remote_offline(node_id));
        }
        let guard = self
            .inner
            .lock()
            .map_err(|_| MinewrapError::offline("node service locked"))?;
        let map = guard
            .instances
            .get(node_id)
            .ok_or_else(|| MinewrapError::gone("no instances on node"))?;
        let rt = map
            .get(instance_id)
            .ok_or_else(|| MinewrapError::gone(format!("instance {instance_id} not found")))?;
        Ok(SnapshotDto {
            id: rt.state.id.clone(),
            name: rt.state.name.clone(),
            combined_state: rt.state.combined_state(),
            process_state: ProcessState::Offline,
            probe_ok: false,
            players_online: None,
            players_max: None,
            version: None,
        })
    }

    pub fn register_probe(&self, node_id: &str, instance_id: &str, probe: ProbeInfo) {
        if !self.is_local(node_id) {
            return;
        }
        if let Ok(mut guard) = self.inner.lock() {
            if let Some(map) = guard.instances.get_mut(node_id) {
                if let Some(rt) = map.get_mut(instance_id) {
                    rt.state.update_probe(probe);
                }
            }
        }
    }
}

/// Flattened snapshot DTO (the full `InstanceSnapshot` ships in Phase 4).
#[derive(Debug, Clone, serde::Serialize)]
pub struct SnapshotDto {
    pub id: String,
    pub name: String,
    pub combined_state: String,
    pub process_state: ProcessState,
    pub probe_ok: bool,
    pub players_online: Option<u32>,
    pub players_max: Option<u32>,
    pub version: Option<String>,
}

impl NodeService {
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
        let _ = (instance_id, limit, since_seq);
        // Phase 4 backs this from the per-instance LogBuffer; the IPC
        // surface is stable from now.
        Ok(Vec::new())
    }

    pub fn describe(&self, note: &str) -> Arc<Self> {
        let _ = note;
        Arc::new(Self::new())
    }
}
