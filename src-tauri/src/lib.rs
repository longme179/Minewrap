//! Minewrap v2 Tauri shell.
//!
//! Thin IPC layer: commands carry `node_id` and are routed by
//! `NodeService` to the embedded `minewrap-core` (local node) or to a
//! remote `minewrapd` client. No core logic lives here — the frontend
//! never touches processes, files, or sockets directly, and remote
//! tokens never enter the webview.

pub mod commands;
pub mod node_service;
pub mod types;

pub use node_service::NodeService;
pub use types::{MinewrapError, NodeState};
