//! minewrapd binary: load config, generate-or-read token (0600),
//! start axum REST+WS on host:port, and run the always-on services:
//! state keeper (B1), SLP poller (B2), Discord pusher (B3).
//! SIGTERM/SIGINT (B5) stops every instance gracefully, then exits.

use std::sync::Arc;

use minewrap_core::{Control, ProcessState};
use minewrapd::{
    InstanceManager, auth::TokenAuth, create_router, discord_service, poller_task, state_keeper,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config_path = std::env::var("MINEWRAP_CONFIG").unwrap_or_else(|_| "minewrap.toml".into());
    let daemon = minewrap_core::DaemonConfig::load_from(&config_path).unwrap_or_else(|e| {
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
                    let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
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

    // Always-on services (P0 daemon-first).
    let _keeper = state_keeper::spawn(mgr.clone());
    let _poller = poller_task::spawn(mgr.clone());
    let _discord = discord_service::spawn(mgr.clone());

    let addr = format!("{}:{}", daemon.daemon.host, daemon.daemon.port);
    println!("minewrapd: listening on {addr} (SSH forward to reach remotely)");
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    // B5: graceful shutdown — SIGTERM/SIGINT stop every instance
    // (stdin `stop` -> SIGTERM -> SIGKILL escalation per instance),
    // then the daemon exits 0. This replaces the v1.9.3 "close the GUI
    // to stop everything" path; `systemctl --user stop minewrapd` and
    // `kill -TERM` both land here.
    let mgr_for_shutdown = mgr.clone();
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            stop_all_instances(&mgr_for_shutdown).await;
        })
        .await?;
    Ok(())
}

/// Resolve on SIGTERM or SIGINT (whichever comes first).
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    let mut int = signal(SignalKind::interrupt()).expect("install SIGINT handler");
    tokio::select! {
        _ = term.recv() => println!("minewrapd: SIGTERM received"),
        _ = int.recv() => println!("minewrapd: SIGINT received"),
    }
}

/// Stop every instance: graceful `Control::Stop` first, wait up to
/// `stop_timeout + kill_grace` for them to settle, then
/// `Control::Shutdown` (kills the process group) for any survivor.
async fn stop_all_instances(mgr: &InstanceManager) {
    let ids = mgr.ids();
    if ids.is_empty() {
        println!("minewrapd: no instances to stop");
        return;
    }
    let (stop_timeout, kill_grace) = {
        let daemon = mgr.daemon.lock().unwrap_or_else(|e| e.into_inner());
        (
            daemon.daemon.defaults.stop_timeout,
            daemon.daemon.defaults.kill_grace_seconds,
        )
    };
    println!(
        "minewrapd: stopping {} instance(s) gracefully (stdin stop -> TERM -> KILL)",
        ids.len()
    );
    for id in &ids {
        mgr.send_control(id, Control::Stop);
    }
    let deadline = tokio::time::Instant::now()
        + std::time::Duration::from_secs_f64(stop_timeout + kill_grace + 2.0);
    loop {
        let unsettled: Vec<&String> = ids
            .iter()
            .filter(|id| {
                matches!(
                    mgr.snapshot_of(id).map(|s| s.process.state),
                    Some(ProcessState::Starting)
                        | Some(ProcessState::Online)
                        | Some(ProcessState::Stopping)
                )
            })
            .collect();
        if unsettled.is_empty() {
            println!("minewrapd: all instances settled");
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            println!(
                "minewrapd: grace expired, force-shutdown {} instance(s)",
                unsettled.len()
            );
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    for id in &ids {
        mgr.send_control(id, Control::Shutdown);
    }
    // Give the Shutdown handlers a beat to kill the groups.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    println!("minewrapd: exiting");
}
