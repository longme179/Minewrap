//! minewrapd — headless axum daemon (REST + WebSocket) for remote
//! nodes. Uses the same `minewrap-core` as the Tauri app; binds
//! 127.0.0.1 by default. SSH port-forward is the documented way to
//! reach it remotely (see docs/remote-access.md).

pub mod api;
pub mod auth;
pub mod instances;

pub use api::{create_router, ws};
pub use instances::InstanceManager;
