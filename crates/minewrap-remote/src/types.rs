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

// --- v1.10.0 (P0 daemon-first): typed DTOs the GUI needs without
// `minewrap-core`. Wire shapes are exactly what minewrapd serves. ---

/// One log line (`{seq, ts, line}` — the frontend `LogLine`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLineDto {
    pub seq: u64,
    /// Wire format is epoch **milliseconds** (`chrono::serde::ts_milliseconds`,
    /// matching `minewrap_core::LogLine`), not an RFC3339 string.
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub ts: chrono::DateTime<chrono::Utc>,
    pub line: String,
}

/// `GET /api/v1/instances` entry (the frontend `InstanceSummary`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceSummaryDto {
    pub id: String,
    pub name: String,
    pub combined_state: String,
    #[serde(default)]
    pub players_online: Option<u32>,
    #[serde(default)]
    pub players_max: Option<u32>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub latency_ms: Option<f64>,
}

/// Flattened snapshot (the frontend `SnapshotDto`): the daemon's nested
/// `InstanceSnapshot` mapped to the shape the webview has always seen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotDto {
    pub id: String,
    pub name: String,
    pub combined_state: String,
    pub process_state: String,
    pub probe_ok: bool,
    #[serde(default)]
    pub players_online: Option<u32>,
    #[serde(default)]
    pub players_max: Option<u32>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
}

impl SnapshotDto {
    /// The offline placeholder the v1.9.3 GUI returned for unknown
    /// instances (UI parity when a 404 arrives).
    pub fn offline_placeholder(id: &str) -> Self {
        Self {
            id: id.to_string(),
            name: id.to_string(),
            combined_state: "offline".to_string(),
            process_state: "offline".to_string(),
            probe_ok: false,
            players_online: None,
            players_max: None,
            version: None,
            last_error: None,
        }
    }
}

/// Raw daemon snapshot JSON (nested) → `SnapshotDto`.
#[derive(Debug, Clone, Deserialize)]
pub struct DaemonSnapshot {
    pub id: String,
    pub name: String,
    pub combined_state: String,
    #[serde(default)]
    pub process: DaemonProcess,
    #[serde(default)]
    pub probe: DaemonProbe,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DaemonProcess {
    #[serde(default)]
    pub state: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DaemonProbe {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub players_online: Option<u32>,
    #[serde(default)]
    pub players_max: Option<u32>,
    #[serde(default)]
    pub version: Option<String>,
}

impl From<DaemonSnapshot> for SnapshotDto {
    fn from(s: DaemonSnapshot) -> Self {
        Self {
            id: s.id,
            name: s.name,
            combined_state: s.combined_state,
            process_state: s.process.state,
            probe_ok: s.probe.ok,
            players_online: s.probe.players_online,
            players_max: s.probe.players_max,
            version: s.probe.version,
            last_error: s.last_error,
        }
    }
}

/// `GET /api/v1/java` entry (the frontend `JavaRuntime`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JavaRuntimeDto {
    pub home: String,
    pub binary: String,
    #[serde(default)]
    pub major: Option<u8>,
    #[serde(default)]
    pub full_version: Option<String>,
    #[serde(default)]
    pub vendor: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
    pub source: String,
    pub is_jdk: bool,
}

/// Local daemon connection settings, parsed from the GUI's
/// `minewrap.toml` (`[daemon]` table only — the GUI is a client and
/// no longer links `minewrap-core`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalSettings {
    pub host: String,
    pub port: u16,
    pub data_dir: String,
}

impl Default for LocalSettings {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 8080,
            data_dir: "./data".into(),
        }
    }
}

/// Parse `[daemon] host/port/data_dir` from a minewrap.toml, resolving
/// a relative `data_dir` against the config file's directory (same
/// semantics as `minewrap_core::DaemonConfig::resolve_paths`).
pub fn load_local_settings(path: &str) -> Result<LocalSettings, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("cannot read config {path}: {e}"))?;
    #[derive(serde::Deserialize, Default)]
    struct DaemonTable {
        #[serde(default = "default_host")]
        host: String,
        #[serde(default = "default_port")]
        port: u16,
        #[serde(default = "default_data_dir")]
        data_dir: String,
    }
    fn default_host() -> String {
        "127.0.0.1".into()
    }
    fn default_port() -> u16 {
        8080
    }
    fn default_data_dir() -> String {
        "./data".into()
    }
    #[derive(serde::Deserialize, Default)]
    struct Doc {
        #[serde(default)]
        daemon: DaemonTable,
    }
    let doc: Doc = toml::from_str(&text).map_err(|e| format!("config parse: {e}"))?;
    let mut settings = LocalSettings {
        host: doc.daemon.host,
        port: doc.daemon.port,
        data_dir: doc.daemon.data_dir,
    };
    let data_dir = std::path::PathBuf::from(&settings.data_dir);
    if data_dir.is_relative() {
        let base = std::path::Path::new(path)
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf();
        settings.data_dir = base.join(data_dir).to_string_lossy().to_string();
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_settings_parses_daemon_table() {
        let dir = std::env::temp_dir().join("minewrap-remote-settings-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("minewrap.toml");
        std::fs::write(
            &path,
            "[daemon]\nhost = \"127.0.0.1\"\nport = 8377\ndata_dir = \"./data\"\n\n[[instances]]\nid = \"x\"\nwork_dir = \"/tmp\"\n",
        )
        .unwrap();
        let s = load_local_settings(path.to_str().unwrap()).unwrap();
        assert_eq!(s.host, "127.0.0.1");
        assert_eq!(s.port, 8377);
        assert_eq!(std::path::Path::new(&s.data_dir), dir.join("./data"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_flattens_daemon_shape() {
        let raw = serde_json::json!({
            "id": "srv", "name": "Srv", "combined_state": "online",
            "process": {"state": "online"},
            "probe": {"ok": true, "players_online": 3, "players_max": 20, "version": "1.21"},
            "updated_at": "2026-11-11T00:00:00Z",
            "last_error": null,
        });
        let snap: SnapshotDto = serde_json::from_value::<DaemonSnapshot>(raw)
            .unwrap()
            .into();
        assert_eq!(snap.process_state, "online");
        assert!(snap.probe_ok);
        assert_eq!(snap.players_online, Some(3));
        assert_eq!(snap.last_error, None);
    }

    #[test]
    fn log_line_dto_parses() {
        let v: LogLineDto = serde_json::from_value(serde_json::json!({
            "seq": 7, "ts": 1794401798153u64, "line": "Done"
        }))
        .unwrap();
        assert_eq!(v.seq, 7);
        assert_eq!(v.line, "Done");
        assert_eq!(v.ts.timestamp_millis(), 1794401798153);
    }
}
