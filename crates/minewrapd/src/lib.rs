//! minewrapd — headless axum daemon (REST + WebSocket). Uses
//! `minewrap-core` and binds 127.0.0.1 by default. SSH port-forward
//! is the documented way to reach it remotely (docs/remote-access.md).
//!
//! v1.10.0 (P0 daemon-first): the daemon is the always-on half — it
//! keeps live instance state (state keeper), probes SLP in the
//! background, pushes Discord notifications, and shuts down its whole
//! process tree on SIGTERM. The GUI is now a pure client.

pub mod api;
pub mod auth;
pub mod discord_service;
pub mod instances;
pub mod poller_task;
pub mod state_keeper;

pub use api::{create_router, ws};
pub use instances::InstanceManager;
