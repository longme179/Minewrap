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
pub mod java;
pub mod launch;
pub mod logs;
pub mod mods;
pub mod poller;
pub mod process;
pub mod properties;

pub use config::{
    DaemonConfig, DaemonSettings, DiscordConfig, InstanceConfig, InstanceDefaults, ProbeConfig,
    RestartConfig,
};
pub use discord::{Notifier, build_state_change_embed, build_status_embed};
pub use events::{
    Event, EventBus, EventBus as EventBusT, LogLineEvent, RestartLoopEvent, StateChange,
};
pub use java::{JavaRuntime, detect_all, min_java_for_minecraft};
pub use launch::{
    LaunchCandidate, LaunchKind, WorkDirReport, inspect_jar, report_work_dir, scan_work_dir,
    suggested_for_mode,
};
pub use logs::{LogBuffer, LogLine, LogLineBatch, RotatingFile};
pub use mods::{ModEntry, format_size, list_mods, toggle_mod};
pub use poller::{InstanceSnapshot, InstanceState, InstanceSummary, Poller, ProbeInfo, probe_once};
pub use process::{
    Control, OwnerHandle, ProcessError, ProcessManager, ProcessState, ProcessStatus, spawn_reader,
    spawn_server,
};
pub use properties::{
    PropertiesDocument, PropertyEntry, confine_path, properties_path, validate_property,
};

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
        assert_eq!(CORE_VERSION, env!("CARGO_PKG_VERSION"));
    }
}
