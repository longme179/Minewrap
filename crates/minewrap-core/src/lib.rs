//! minewrap-core — Tauri-free server core shared by the desktop app and `minewrapd`.
//!
//! Phase 1 delivers: config, process lifecycle/state machine, log pipeline
//! (batching + ring buffer + rotation), and a fake server fixture. The
//! poller, Discord, properties editor and mods manager land in Phase 2.

pub mod config;
pub mod events;
pub mod logs;
pub mod process;

pub use config::{
    DaemonConfig, DaemonSettings, InstanceConfig, InstanceDefaults, ProbeConfig, RestartConfig,
};
pub use logs::{LogBuffer, LogLine, LogLineBatch};
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
