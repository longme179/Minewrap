//! Minewrap v2 Tauri shell — pure client (v1.10.0 P0 daemon-first).
//!
//! Thin IPC layer: commands carry `node_id` and are routed by
//! `NodeService` to a `minewrapd` over `minewrap-remote` (local =
//! 127.0.0.1 daemon, auto-spawned by `main.rs`). No server logic lives
//! here: closing the window only disconnects; the frontend never
//! touches processes, files, or sockets directly, and remote tokens
//! never enter the webview.

pub mod commands;
pub mod node_service;
pub mod types;

pub use node_service::NodeService;
pub use types::{MinewrapError, NodeState};
