//! minewrapd binary: load config, generate-or-read token (0600),
//! start axum REST+WS on host:port, embed the same core as the GUI.

use std::sync::Arc;

use minewrapd::{InstanceManager, auth::TokenAuth, create_router};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config_path = std::env::var("MINEWRAP_CONFIG").unwrap_or_else(|_| "minewrap.toml".into());
    let daemon = minewrap_core::DaemonConfig::load_from(&config_path)
        .unwrap_or_else(|e| {
            eprintln!("minewrapd: failed to load {config_path}: {e}; using defaults");
            minewrap_core::DaemonConfig::default()
        });

    // Token: explicit env wins, data/token file next (generated once,
    // 0600), otherwise generate and persist (v1 config_store parity).
    let data_dir = std::path::PathBuf::from(&daemon.daemon.data_dir);
    std::fs::create_dir_all(&data_dir).ok();
    let token_path = data_dir.join("token");
    let token = std::env::var("MINEWRAP_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if let Ok(t) = std::fs::read_to_string(&token_path) {
                t.trim().to_string()
            } else {
                let fresh = TokenAuth::generate();
                std::fs::create_dir_all(&data_dir).ok();
                let tmp = token_path.with_extension("token.tmp");
                std::fs::write(&tmp, &fresh).ok();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(
                        &tmp,
                        std::fs::Permissions::from_mode(0o600),
                    );
                }
                std::fs::rename(&tmp, &token_path).ok();
                println!("minewrapd: generated token, printed once: {fresh}");
                fresh
            }
        });

    let bus = Arc::new(minewrap_core::EventBus::new());
    let mgr = Arc::new(InstanceManager::load(
        daemon.clone(),
        bus,
        Some(config_path.clone()),
    ));
    let auth = TokenAuth::new(token);
    let router = create_router(mgr.clone(), auth);

    let addr = format!("{}:{}", daemon.daemon.host, daemon.daemon.port);
    println!("minewrapd: listening on {addr} (SSH forward to reach remotely)");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, router).await?;
    Ok(())
}
