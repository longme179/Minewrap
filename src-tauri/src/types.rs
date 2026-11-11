//! Shared IPC DTOs + error envelope.
//!
//! v1.10.0: the shapes live in `minewrap-remote` (single source shared
//! by the daemon client and the shell); this module only re-exports
//! them so `crate::types::…` keeps working.

pub use minewrap_remote::types::{
    Action, DiscordTestResponse, HealthResponse, InstanceSummaryDto, JavaRuntimeDto, LocalSettings,
    LogLineDto, MinewrapError, ModDto, NodeState, NodeSummary, PropertiesDoc, SnapshotDto,
};
