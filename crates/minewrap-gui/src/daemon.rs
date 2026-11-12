//! Local daemon discovery — config host/port/data_dir, /health probe,
//! detached minewrapd auto-spawn, bearer token from {data_dir}/token.
//! `MINEWRAP_CONFIG` or `~/.config/minewrap/minewrap.toml` → host/port/data_dir,
//! `/health` probe, detached `minewrapd` auto-spawn, bearer token from
//! `{data_dir}/token`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use minewrap_remote::RemoteClient;
use minewrap_remote::types::{LocalSettings, load_local_settings};

/// Where the local daemon listens and which token unlocks it.
pub struct LocalDaemon {
    pub config_path: PathBuf,
    pub settings: LocalSettings,
    pub token: String,
}

/// `~/.config/minewrap` (XDG-aware).
pub fn config_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(path).join("minewrap");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".config").join("minewrap");
    }
    PathBuf::from(".").join(".minewrap")
}

/// `MINEWRAP_CONFIG` if set, else the stable XDG path (created with
/// defaults on first run so both halves agree on host/port/data_dir).
pub fn default_config_path() -> PathBuf {
    if let Some(path) = std::env::var_os("MINEWRAP_CONFIG") {
        return PathBuf::from(path);
    }
    let dir = config_dir();
    let _ = std::fs::create_dir_all(&dir);
    let stable = dir.join("minewrap.toml");
    if !stable.exists() {
        let _ = std::fs::write(
            &stable,
            "[daemon]\nhost = \"127.0.0.1\"\nport = 8080\ndata_dir = \"./data\"\n",
        );
    }
    stable
}

pub fn base_url(settings: &LocalSettings) -> String {
    format!("http://{}:{}", settings.host, settings.port)
}

/// Bearer token written by the daemon (0600); empty when not there yet.
pub fn read_token(data_dir: &str) -> String {
    std::fs::read_to_string(Path::new(data_dir).join("token"))
        .map(|t| t.trim().to_string())
        .unwrap_or_default()
}

/// `minewrap.toml` → settings; a broken/missing config falls back to the
/// same defaults the daemon uses.
pub fn load_settings(config_path: &Path) -> LocalSettings {
    load_local_settings(&config_path.to_string_lossy()).unwrap_or_else(|e| {
        eprintln!(
            "minewrap-gui: cannot load {}: {e}; using defaults",
            config_path.display()
        );
        LocalSettings::default()
    })
}

/// Locate `minewrapd`: `MINEWRAPD_BIN`, next to the GUI binary, then PATH.
pub fn find_daemon_binary() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("MINEWRAPD_BIN") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let mut candidates = vec![
        dir.join("minewrapd"),
        dir.join("binaries").join("minewrapd"),
        dir.join("binaries")
            .join("minewrapd-x86_64-unknown-linux-gnu"),
    ];
    if let Ok(path_var) = std::env::var("PATH") {
        for p in path_var.split(':') {
            if !p.is_empty() {
                candidates.push(PathBuf::from(p).join("minewrapd"));
            }
        }
    }
    candidates.into_iter().find(|c| c.exists())
}

/// Result of the startup daemon check.
pub enum Startup {
    AlreadyRunning,
    Spawned,
    Unavailable(String),
}

/// Probe `/health`; spawn the daemon detached if down, then wait <= 10 s.
pub fn ensure_daemon(
    rt: &tokio::runtime::Runtime,
    config_path: &Path,
    settings: &LocalSettings,
) -> Startup {
    let base = base_url(settings);
    if probe(rt, &base, &settings.data_dir) {
        return Startup::AlreadyRunning;
    }
    let Some(binary) = find_daemon_binary() else {
        return Startup::Unavailable(
            "minewrapd not found next to the GUI or on PATH (set MINEWRAPD_BIN)".into(),
        );
    };
    let data_dir = Path::new(&settings.data_dir);
    if let Err(e) = std::fs::create_dir_all(data_dir) {
        return Startup::Unavailable(format!("data dir: {e}"));
    }
    let log_path = data_dir.join("daemon.log");
    let stdout = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        Ok(f) => f,
        Err(e) => return Startup::Unavailable(format!("open {}: {e}", log_path.display())),
    };
    let stderr = match stdout.try_clone() {
        Ok(f) => f,
        Err(e) => return Startup::Unavailable(format!("clone log handle: {e}")),
    };
    let mut command = Command::new(&binary);
    command
        .env("MINEWRAP_CONFIG", config_path)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0); // setsid: outlive the GUI (closing it stops nothing)
    }
    if let Err(e) = command.spawn() {
        return Startup::Unavailable(format!("spawn {}: {e}", binary.display()));
    }
    eprintln!(
        "minewrap-gui: spawned daemon {} (config {}, log {})",
        binary.display(),
        config_path.display(),
        log_path.display()
    );
    // The fresh daemon may (re)generate the token, so re-read it per poll.
    for _ in 0..40 {
        if probe(rt, &base, &settings.data_dir) {
            return Startup::Spawned;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    Startup::Unavailable("daemon did not become healthy within 10 s".into())
}

fn probe(rt: &tokio::runtime::Runtime, base: &str, data_dir: &str) -> bool {
    let client = RemoteClient::new(base.to_string(), read_token(data_dir));
    rt.block_on(async {
        client
            .health_timeout(std::time::Duration::from_secs(1))
            .await
            .is_ok()
    })
}
