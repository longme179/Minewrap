//! minewrap-core — Tauri-free server core shared by the desktop app and `minewrapd`.
//!
//! Implements the full v1 parity core: config, process/state machine, log
//! pipeline (batching, ring buffer, rotation), SLP status probe, Discord
//! notifier (including redaction/retry), server.properties, mods and path
//! confinement. Effectively Tauri-free: sending the same HTTP/WebSocket wire
//! format as `minewrapd`.

pub mod config;
pub mod discard;
pub mod discord;
pub mod events;
pub mod logs;
pub mod mods;
pub mod poller;
pub mod process;
pub mod properties;

pub use config::{
    DaemonConfig, DaemonSettings, InstanceConfig, InstanceDefaults, ProbeConfig, RestartConfig,
};
pub use logs::{LogBuffer, LogLine, LogLineBatch};
pub use poller::{InstanceSnapshot, InstanceState, InstanceSummary, ProbeInfo};
pub use process::{ProcessError, ProcessManager, ProcessState, ProcessStatus};

/// API contract version reported by `minewrapd` (v1.0.0 Python API was "0.1").
pub const API_VERSION: &str = "1.0";

/// Application version of the core crate.
pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_version_is_explicit() {
        assert_eq!(API_VERSION, "1.0");
        assert_eq!(CORE_VERSION, "0.1.0");
    }
}
