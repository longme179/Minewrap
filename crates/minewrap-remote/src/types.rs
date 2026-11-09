//! Shared DTOs for remote + Tauri (mirror of `src-tauri/src/types.rs`).
//! The Tauri `MinewrapError` is defined here so both crates share one
//! error envelope; `src-tauri` re-exports these types.

use serde::{Deserialize, Serialize};

/// Error envelope returned by every command. Codes match ARCHITECTURE.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinewrapError {
    pub code: String,
    pub message: String,
}

impl MinewrapError {
    pub fn offline(message: impl Into<String>) -> Self {
        Self {
            code: "offline".into(),
            message: message.into(),
        }
    }

    pub fn unauthorised(message: impl Into<String>) -> Self {
        Self {
            code: "unauthorised".into(),
            message: message.into(),
        }
    }

    pub fn incompatible(message: impl Into<String>) -> Self {
        Self {
            code: "incompatible_version".into(),
            message: message.into(),
        }
    }

    pub fn gone(message: impl Into<String>) -> Self {
        Self {
            code: "gone".into(),
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "invalid".into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for MinewrapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for MinewrapError {}

/// Well-known local node id (the embedded core).
pub const LOCAL_NODE_ID: &str = "local";

/// Remote node record (UI-facing: the token is never included).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSummary {
    pub id: String,
    pub name: String,
    pub url: String,
    pub is_local: bool,
    pub state: NodeState,
}

/// Connection state machine for a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeState {
    Connecting,
    Connected,
    Offline,
    Unauthorised,
    Incompatible,
}

/// Generic action acknowledgement (start/stop/restart/toggle).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    pub ok: bool,
    pub detail: Option<String>,
}

/// `POST /discord/test` result (secrets-safe).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordTestResponse {
    pub ok: bool,
    pub error: Option<String>,
}

/// Properties document DTO (client <-> core).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertiesDoc {
    pub text: String,
    pub errors: Vec<String>,
}

/// Mod entry DTO.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModDto {
    pub name: String,
    pub size: u64,
    pub enabled: bool,
}

/// `GET /health` body (v1 parity).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub api_version: String,
    pub daemon_version: String,
}
