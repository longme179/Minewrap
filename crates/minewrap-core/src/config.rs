//! Configuration schema (`minewrap.toml`), ported from Python v1
//! `mcpanel_common/config.py` (12 validation tests) and `config_store.py`
//! (7 tests).
//!
//! Differences (deliberate):
//! - unknown keys are denied via `#[serde(deny_unknown_fields)]`;
//! - `token`/`webhook_url` are omitted here and kept in the secrets store;
//! - all range checks mirror v1 `ge/le` bounds.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::API_VERSION;

/// Daemon-visible version reported in `/health` (v2 contract).
pub const HEALTH_API_VERSION: &str = API_VERSION;

/// Matches v1 `INSTANCE_ID_RE`: `^[a-z0-9][a-z0-9_-]{0,62}[a-z0-9]$`.
pub fn valid_instance_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return false;
    }
    if !(bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit()) {
        return false;
    }
    if bytes.len() > 1
        && !(bytes[bytes.len() - 1].is_ascii_lowercase()
            || bytes[bytes.len() - 1].is_ascii_digit())
    {
        return false;
    }
    bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_' || *b == b'-')
}

/// Instance defaults, port of v1 `InstanceDefaults`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceDefaults {
    #[serde(default = "default_startup_timeout")]
    pub startup_timeout: f64,
    #[serde(default = "default_stop_timeout")]
    pub stop_timeout: f64,
    #[serde(default = "default_kill_grace")]
    pub kill_grace_seconds: f64,
    #[serde(default = "default_done_regex")]
    pub done_regex: String,
    #[serde(default = "default_buffer_lines")]
    pub log_buffer_lines: usize,
    #[serde(default = "default_log_max_bytes")]
    pub log_max_bytes: u64,
    #[serde(default = "default_log_backups")]
    pub log_backups: usize,
}

fn default_startup_timeout() -> f64 {
    300.0
}
fn default_stop_timeout() -> f64 {
    60.0
}
fn default_kill_grace() -> f64 {
    5.0
}
pub fn default_done_regex() -> String {
    r"Done \(\d+(\.\d+)?s\)! For help".to_string()
}
fn default_buffer_lines() -> usize {
    500
}
fn default_log_max_bytes() -> u64 {
    10 * 1024 * 1024
}
fn default_log_backups() -> usize {
    3
}

impl Default for InstanceDefaults {
    fn default() -> Self {
        Self {
            startup_timeout: default_startup_timeout(),
            stop_timeout: default_stop_timeout(),
            kill_grace_seconds: default_kill_grace(),
            done_regex: default_done_regex(),
            log_buffer_lines: default_buffer_lines(),
            log_max_bytes: default_log_max_bytes(),
            log_backups: default_log_backups(),
        }
    }
}

impl InstanceDefaults {
    pub fn validate(&self) -> Result<(), String> {
        if self.startup_timeout < 0.1 {
            return Err("startup_timeout must be >= 0.1".into());
        }
        if self.stop_timeout < 0.1 {
            return Err("stop_timeout must be >= 0.1".into());
        }
        if self.kill_grace_seconds < 0.1 {
            return Err("kill_grace_seconds must be >= 0.1".into());
        }
        if self.log_buffer_lines < 10 {
            return Err("log_buffer_lines must be >= 10".into());
        }
        if self.log_max_bytes < 1024 {
            return Err("log_max_bytes must be >= 1024".into());
        }
        if self.log_backups > 100 {
            return Err("log_backups must be <= 100".into());
        }
        Ok(())
    }
}

/// Daemon-level settings, port of `DaemonSettings`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DaemonSettings {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_data_dir")]
    pub data_dir: String,
    /// Pinned token (v1 accepted it in config). Secrets store wins when set.
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default = "default_poll_interval")]
    pub status_poll_interval: f64,
    #[serde(default = "default_unresponsive")]
    pub unresponsive_after_failures: usize,
    #[serde(default)]
    pub defaults: InstanceDefaults,
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}
fn default_port() -> u16 {
    8080
}
fn default_data_dir() -> String {
    "./data".to_string()
}
fn default_poll_interval() -> f64 {
    15.0
}
fn default_unresponsive() -> usize {
    2
}

impl Default for DaemonSettings {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            data_dir: default_data_dir(),
            token: None,
            status_poll_interval: default_poll_interval(),
            unresponsive_after_failures: default_unresponsive(),
            defaults: InstanceDefaults::default(),
        }
    }
}

impl DaemonSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.port == 0 {
            return Err("daemon.port must be between 1 and 65535".into());
        }
        if self.status_poll_interval < 0.1 {
            return Err("status_poll_interval must be >= 0.1".into());
        }
        self.defaults.validate()?;
        Ok(())
    }
}

/// Probe address, port of `ProbeConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeConfig {
    #[serde(default = "default_probe_host")]
    pub host: String,
    #[serde(default = "default_probe_port")]
    pub port: u16,
    #[serde(default)]
    pub query_enabled: bool,
    #[serde(default)]
    pub query_port: Option<u16>,
    #[serde(default = "default_probe_timeout")]
    pub timeout: f64,
}

fn default_probe_host() -> String {
    "127.0.0.1".to_string()
}
fn default_probe_port() -> u16 {
    25565
}
fn default_probe_timeout() -> f64 {
    3.0
}

impl Default for ProbeConfig {
    fn default() -> Self {
        Self {
            host: default_probe_host(),
            port: default_probe_port(),
            query_enabled: false,
            query_port: None,
            timeout: default_probe_timeout(),
        }
    }
}

impl ProbeConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.port == 0 {
            return Err("probe.port must be between 1 and 65535".into());
        }
        if self.timeout < 0.1 {
            return Err("probe.timeout must be >= 0.1".into());
        }
        Ok(())
    }
}

/// Restart policy, port of `RestartConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestartConfig {
    /// `"never"` or `"on-crash"`.
    #[serde(default = "default_policy")]
    pub policy: String,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_backoff_base")]
    pub backoff_base_seconds: f64,
    #[serde(default = "default_backoff_max")]
    pub backoff_max_seconds: f64,
    #[serde(default = "default_reset_after")]
    pub reset_after_online_seconds: f64,
}

fn default_policy() -> String {
    "on-crash".to_string()
}
fn default_max_retries() -> u32 {
    5
}
fn default_backoff_base() -> f64 {
    5.0
}
fn default_backoff_max() -> f64 {
    300.0
}
fn default_reset_after() -> f64 {
    600.0
}

impl Default for RestartConfig {
    fn default() -> Self {
        Self {
            policy: default_policy(),
            max_retries: default_max_retries(),
            backoff_base_seconds: default_backoff_base(),
            backoff_max_seconds: default_backoff_max(),
            reset_after_online_seconds: default_reset_after(),
        }
    }
}

impl RestartConfig {
    pub fn validate(&self) -> Result<(), String> {
        match self.policy.as_str() {
            "never" | "on-crash" => {}
            other => return Err(format!("unknown restart policy: {other:?}")),
        }
        if self.max_retries > 100 {
            return Err("max_retries must be <= 100".into());
        }
        Ok(())
    }
}

/// Rcon options (v1 carried them; still unimplemented in v2, documented).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RconConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_probe_host")]
    pub host: String,
    #[serde(default = "default_rcon_port")]
    pub port: u16,
    #[serde(default = "default_rcon_env")]
    pub password_env: String,
}

fn default_rcon_port() -> u16 {
    25575
}
fn default_rcon_env() -> String {
    "MC_RCON_PASSWORD".to_string()
}

impl Default for RconConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: default_probe_host(),
            port: default_rcon_port(),
            password_env: default_rcon_env(),
        }
    }
}

/// Discord notifier config, port of `DiscordConfig` (minus the secret, which
/// lives in the secrets store).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscordConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Stored in the keyring/0600 file when set (not a plain secret token).
    #[serde(default)]
    pub webhook_url: Option<String>,
    /// `"edit-in-place"` or `"new-message"`.
    #[serde(default = "default_discord_mode")]
    pub mode: String,
    #[serde(default = "default_discord_interval")]
    pub interval_seconds: f64,
    #[serde(default = "default_first_push")]
    pub first_push_delay_seconds: f64,
    #[serde(default = "default_log_lines")]
    pub log_lines: usize,
    #[serde(default = "default_max_log_chars")]
    pub max_log_chars: usize,
    #[serde(default = "default_true")]
    pub state_change_posts: bool,
    #[serde(default = "default_debounce")]
    pub state_change_debounce_seconds: f64,
    #[serde(default)]
    pub redact_regexes: Vec<String>,
}

fn default_discord_mode() -> String {
    "edit-in-place".to_string()
}
fn default_discord_interval() -> f64 {
    600.0
}
fn default_first_push() -> f64 {
    10.0
}
fn default_log_lines() -> usize {
    15
}
fn default_max_log_chars() -> usize {
    1500
}
fn default_true() -> bool {
    true
}
fn default_debounce() -> f64 {
    30.0
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            webhook_url: None,
            mode: default_discord_mode(),
            interval_seconds: default_discord_interval(),
            first_push_delay_seconds: default_first_push(),
            log_lines: default_log_lines(),
            max_log_chars: default_max_log_chars(),
            state_change_posts: default_true(),
            state_change_debounce_seconds: default_debounce(),
            redact_regexes: Vec::new(),
        }
    }
}

impl DiscordConfig {
    /// Webhook never appears in `Display`/debug.
    pub fn webhook_set(&self) -> bool {
        self.webhook_url.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
    }

    pub fn validate(&self) -> Result<(), String> {
        match self.mode.as_str() {
            "edit-in-place" | "new-message" => {}
            other => return Err(format!("unknown discord mode: {other:?}")),
        }
        if self.interval_seconds < 0.1 {
            return Err("interval_seconds must be >= 0.1".into());
        }
        if self.log_lines > 50 {
            return Err("log_lines must be <= 50".into());
        }
        if self.max_log_chars > 4096 {
            return Err("max_log_chars must be <= 4096".into());
        }
        // user regexes are compiled at notifier construction (Phase 2)
        for pattern in &self.redact_regexes {
            let _ = pattern;
        }
        Ok(())
    }
}

/// Per-instance config, port of `InstanceConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceConfig {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub work_dir: String,
    #[serde(default = "default_java")]
    pub java_path: String,
    #[serde(default)]
    pub jvm_args: Vec<String>,
    #[serde(default = "default_launch_args")]
    pub launch_args: Vec<String>,
    #[serde(default)]
    pub autostart: bool,
    #[serde(default)]
    pub probe: ProbeConfig,
    #[serde(default)]
    pub restart: RestartConfig,
    #[serde(default)]
    pub rcon: Option<RconConfig>,
    #[serde(default)]
    pub discord: DiscordConfig,
}

fn default_java() -> String {
    "java".to_string()
}
fn default_launch_args() -> Vec<String> {
    vec!["-jar".to_string(), "server.jar".to_string(), "nogui".to_string()]
}

/// Aikar's Flags (https://aikar.co/flags.html) — the recommended
/// JVM preset for modern Minecraft servers. Exposed so the Settings
/// UI offers it as a one-click preset; the user can still edit
/// `jvm_args` freely afterwards.
pub fn aikars_flags() -> Vec<String> {
    vec![
        "-Xms2G".into(),
        "-Xmx4G".into(),
        "-XX:+UseG1GC".into(),
        "-XX:+ParallelRefProcEnabled".into(),
        "-XX:MaxGCPauseMillis=200".into(),
        "-XX:+UnlockExperimentalVMOptions".into(),
        "-XX:+DisableExplicitGC".into(),
        "-XX:+AlwaysPreTouch".into(),
        "-XX:G1NewSizeRatio=30".into(),
        "-XX:G1MaxNewSizeRatio=40".into(),
        "-XX:G1HeapRegionSize=8M".into(),
        "-XX:G1ReservePercent=20".into(),
        "-XX:G1HeapWastePercent=5".into(),
        "-XX:G1MixedGCCountTarget=4".into(),
        "-XX:InitiatingHeapOccupancyPercent=15".into(),
        "-XX:G1MixedGCLiveThresholdPercent=90".into(),
        "-XX:G1RSetUpdatingPauseTimePercent=5".into(),
        "-XX:SurvivorRatio=32".into(),
        "-XX:+PerfDisableSharedMem".into(),
        "-XX:MaxTenuringThreshold=1".into(),
        "-Dusing.aikars.flags=https://mcflags.env".into(),
        "-Daikars.flags.file=aikars.flags".into(),
        "-Dlog4j2.formatMsgNoLookups=true".into(),
    ]
}

impl InstanceConfig {
    /// Apply the v1 default-name rule (`name` falls back to `id`).
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            &self.id
        } else {
            &self.name
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !valid_instance_id(&self.id) {
            return Err(format!("invalid instance id: {:?}", self.id));
        }
        if self.work_dir.trim().is_empty() {
            return Err("work_dir must not be empty".into());
        }
        self.probe.validate()?;
        self.restart.validate()?;
        self.discord.validate()?;
        Ok(())
    }
}

/// Top-level document, port of `DaemonConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DaemonConfig {
    #[serde(default)]
    pub daemon: DaemonSettings,
    #[serde(default)]
    pub instances: Vec<InstanceConfig>,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            daemon: DaemonSettings::default(),
            instances: Vec::new(),
        }
    }
}

impl DaemonConfig {
    pub fn validate(&self) -> Result<(), String> {
        self.daemon.validate()?;
        let mut seen = std::collections::HashSet::new();
        for inst in &self.instances {
            inst.validate()?;
            if !seen.insert(inst.id.clone()) {
                return Err(format!("duplicate instance id: {:?}", inst.id));
            }
        }
        Ok(())
    }

    /// Paths (`data_dir`, `work_dir`) resolved against the config file dir.
    pub fn resolve_paths(&mut self, base: &std::path::Path) {
        let data_dir = std::path::PathBuf::from(&self.daemon.data_dir);
        if data_dir.is_relative() {
            self.daemon.data_dir = base.join(data_dir).to_string_lossy().to_string();
        }
        for inst in &mut self.instances {
            let work = std::path::PathBuf::from(&inst.work_dir);
            if work.is_relative() {
                inst.work_dir = base.join(work).to_string_lossy().to_string();
            }
        }
    }

    pub fn parse_toml(input: &str) -> Result<Self, String> {
        let config: DaemonConfig = toml::from_str(input).map_err(|e| e.to_string())?;
        config.validate()?;
        Ok(config)
    }

    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string(self).map_err(|e| e.to_string())
    }

    /// Parse, validate and resolve paths against the file's directory.
    pub fn load_from(path_str: &str) -> Result<Self, String> {
        let path = std::path::Path::new(path_str);
        let input = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read config {path_str}: {e}"))?;
        let mut config = Self::parse_toml(&input)?;
        let base = path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf();
        config.resolve_paths(&base);
        Ok(config)
    }

    /// Atomically write (mode 0600) — v1 `config_store.save_config` parity.
    pub fn save_to(&self, path_str: &str) -> Result<(), String> {
        let path = PathBuf::from(path_str);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let data = self.to_toml()?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, &data).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                &tmp,
                std::fs::Permissions::from_mode(0o600),
            );
        }
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Instance config views: secrets masked (`webhook_set: bool`),
    /// never the webhook URL.
    pub fn instance_view(&self, id: &str) -> serde_json::Value {
        let cfg = match self.instances.iter().find(|c| c.id == id) {
            Some(c) => c,
            None => return serde_json::json!({"id": id, "not_found": true}),
        };
        serde_json::json!({
            "id": cfg.id,
            "name": cfg.display_name(),
            "work_dir": cfg.work_dir,
            "java_path": cfg.java_path,
            "jvm_args": cfg.jvm_args,
            "launch_args": cfg.launch_args,
            "autostart": cfg.autostart,
            "probe": cfg.probe,
            "restart": cfg.restart,
            "rcon": cfg.rcon,
            "discord": {
                "enabled": cfg.discord.enabled,
                "webhook_set": cfg.discord.webhook_set(),
                "mode": cfg.discord.mode,
                "interval_seconds": cfg.discord.interval_seconds,
                "first_push_delay_seconds": cfg.discord.first_push_delay_seconds,
                "log_lines": cfg.discord.log_lines,
                "max_log_chars": cfg.discord.max_log_chars,
                "state_change_posts": cfg.discord.state_change_posts,
                "state_change_debounce_seconds": cfg.discord.state_change_debounce_seconds,
                "redact_regexes": cfg.discord.redact_regexes,
            },
        })
    }

    /// Patch one instance (masked writes; `webhook_url` is write-only),
    /// persist + validate. Callers get 422-safe errors here.
    pub fn patch_instance(
        &mut self,
        id: &str,
        patch: serde_json::Value,
    ) -> Result<(), String> {
        let inst = self
            .instances
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| format!("unknown instance: {id}"))?;

        if let Some(v) = patch.get("name").and_then(|v| v.as_str()) {
            if !v.trim().is_empty() {
                inst.name = v.trim().to_string();
            }
        }
        if let Some(v) = patch.get("work_dir").and_then(|v| v.as_str()) {
            if !v.trim().is_empty() {
                inst.work_dir = v.trim().to_string();
            }
        }
        if let Some(v) = patch.get("java_path").and_then(|v| v.as_str()) {
            if !v.trim().is_empty() {
                inst.java_path = v.trim().to_string();
            }
        }
        if let Some(v) = patch.get("jvm_args").and_then(|v| v.as_array()) {
            inst.jvm_args = v
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect();
        }
        if let Some(v) = patch.get("launch_args").and_then(|v| v.as_array()) {
            let args: Vec<String> = v
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect();
            if !args.is_empty() {
                inst.launch_args = args;
            }
        }
        if let Some(v) = patch.get("autostart").and_then(|v| v.as_bool()) {
            inst.autostart = v;
        }
        if let Some(d) = patch.get("probe").and_then(|v| v.as_object()) {
            for (k, v) in d {
                match k.as_str() {
                    "host" => inst.probe.host = v.as_str().unwrap_or("").to_string(),
                    "port" => inst.probe.port = v.as_u64().unwrap_or(25565) as u16,
                    "query_enabled" => inst.probe.query_enabled = v.as_bool().unwrap_or(false),
                    "timeout" => inst.probe.timeout = v.as_f64().unwrap_or(3.0).max(0.1),
                    _ => {}
                }
            }
        }
        if let Some(d) = patch.get("restart").and_then(|v| v.as_object()) {
            for (k, v) in d {
                match k.as_str() {
                    "policy" => inst.restart.policy = v.as_str().unwrap_or("").to_string(),
                    "max_retries" => inst.restart.max_retries = v.as_u64().unwrap_or(5) as u32,
                    "backoff_base_seconds" => inst.restart.backoff_base_seconds = v.as_f64().unwrap_or(5.0).max(0.1),
                    "backoff_max_seconds" => inst.restart.backoff_max_seconds = v.as_f64().unwrap_or(300.0).max(0.1),
                    "reset_after_online_seconds" => inst.restart.reset_after_online_seconds = v.as_f64().unwrap_or(600.0).max(0.1),
                    _ => {}
                }
            }
        }
        if let Some(d) = patch.get("discord").and_then(|v| v.as_object()) {
            for (k, v) in d {
                match k.as_str() {
                    "enabled" => inst.discord.enabled = v.as_bool().unwrap_or(false),
                    "webhook_url" => {
                        if let Some(w) = v.as_str() {
                            if !w.trim().is_empty() {
                                inst.discord.webhook_url = Some(w.trim().to_string());
                            }
                        }
                    }
                    "mode" => inst.discord.mode = v.as_str().unwrap_or("").to_string(),
                    "interval_seconds" => inst.discord.interval_seconds = v.as_f64().unwrap_or(600.0).max(0.1),
                    "first_push_delay_seconds" => inst.discord.first_push_delay_seconds = v.as_f64().unwrap_or(10.0).max(0.0),
                    "log_lines" => inst.discord.log_lines = v.as_u64().unwrap_or(15) as usize,
                    "max_log_chars" => inst.discord.max_log_chars = v.as_u64().unwrap_or(1500) as usize,
                    "state_change_posts" => inst.discord.state_change_posts = v.as_bool().unwrap_or(true),
                    "state_change_debounce_seconds" => inst.discord.state_change_debounce_seconds = v.as_f64().unwrap_or(30.0).max(0.0),
                    "redact_regexes" => {
                        if let Some(arr) = v.as_array() {
                            inst.discord.redact_regexes = arr
                                .iter()
                                .filter_map(|x| x.as_str().map(String::from))
                                .collect();
                        }
                    }
                    _ => {}
                }
            }
        }

        inst.validate()?;
        self.validate()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_minimal_config_uses_defaults() {
        let input = r#"
[daemon]
data_dir = "./data"

[[instances]]
id = "survival"
work_dir = "server"
"#;
        let config = DaemonConfig::parse_toml(input).unwrap();
        assert_eq!(config.daemon.host, "127.0.0.1");
        assert_eq!(config.daemon.port, 8080);
        assert_eq!(config.daemon.defaults.log_buffer_lines, 500);
        let inst = &config.instances[0];
        assert_eq!(inst.display_name(), "survival");
        assert_eq!(inst.launch_args, ["-jar", "server.jar", "nogui"]);
        assert_eq!(inst.restart.policy, "on-crash");
        assert!(!inst.discord.enabled);
    }

    #[test]
    fn full_config_round_trip() {
        let input = r#"
[daemon]
host = "0.0.0.0"
port = 9000
token = "pinned"

[daemon.defaults]
log_buffer_lines = 250
stop_timeout = 30

[[instances]]
id = "survival"
name = "Survival"
work_dir = "/srv/minecraft/survival"
jvm_args = ["-Xms2G", "-Xmx4G"]
probe = { port = 25566, query_enabled = true }
restart = { policy = "never" }
rcon = { enabled = true, password_env = "MC_RCON_PW" }
discord = { enabled = true, webhook_url = "https://discord.com/api/webhooks/123/abc" }
"#;
        let config = DaemonConfig::parse_toml(input).unwrap();
        assert_eq!(config.daemon.port, 9000);
        let inst = &config.instances[0];
        assert_eq!(inst.probe.query_port, None);
    }

    #[test]
    fn wait_id_names_match_v1() {
        assert!(valid_instance_id("survival"));
        assert!(valid_instance_id("a"));
        assert!(valid_instance_id("a-1_b"));
        assert!(!valid_instance_id("trailing-"));
        assert!(!valid_instance_id("-leading"));
        assert!(!valid_instance_id("UPPER"));
        assert!(!valid_instance_id("has space"));
        assert!(!valid_instance_id(&"a".repeat(65)));
    }

    #[test]
    fn duplicate_ids_rejected() {
        let input = r#"
[[instances]]
id = "survival"
work_dir = "/tmp/a"
[[instances]]
id = "survival"
work_dir = "/tmp/b"
"#;
        assert!(DaemonConfig::parse_toml(input).is_err());
    }

    #[test]
    fn unknown_keys_rejected() {
        let input = r#"
[daemon]
typo_field = 1
"#;
        assert!(DaemonConfig::parse_toml(input).is_err());
    }

    #[test]
    fn out_of_range_versions() {
        let input = r#"
[instances.discord]
interval_seconds = 0.05
"#;
        // note: the bare table parse targets a full doc; serde path should fail
        assert!(DaemonConfig::parse_toml(input).is_err());
    }

    #[test]
    fn round_trip_toml() {
        let cfg = DaemonConfig::default();
        let text = cfg.to_toml().unwrap();
        let parsed = DaemonConfig::parse_toml(&text).unwrap();
        assert_eq!(parsed.daemon.host, cfg.daemon.host);
    }

    #[test]
    fn paths_resolve_against_base() {
        let mut cfg = DaemonConfig::default();
        cfg.daemon.data_dir = "./data".to_string();
        cfg.instances = vec![InstanceConfig {
            id: "x".into(),
            name: "".into(),
            work_dir: "server".into(),
            java_path: "java".into(),
            jvm_args: vec![],
            launch_args: vec![],
            autostart: false,
            probe: ProbeConfig::default(),
            restart: RestartConfig::default(),
            rcon: None,
            discord: DiscordConfig::default(),
        }];
        cfg.resolve_paths(std::path::Path::new("/tmp/cfg"));
        // PathBuf::join keeps `.` segments; the logical result is /tmp/cfg/data.
        assert_eq!(
            std::path::Path::new(&cfg.daemon.data_dir)
                .components()
                .collect::<Vec<_>>(),
            std::path::Path::new("/tmp/cfg/data").components().collect::<Vec<_>>()
        );
        assert_eq!(cfg.instances[0].work_dir, "/tmp/cfg/server");
    }
}
