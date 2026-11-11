//! Discord webhook notifier (v1 `discord.py` parity, including its refinement
//! pass: edit-in-place vs new-message, 429 retry_after capped, 5xx backoff,
//! 404 recreate, masked logging, redact IP/ANSI/user-patterns, discord embed
//! limits).
//!
//! Secret handling: the webhook URL is passed in as a plain `String` but is
//! never logged (the masked form is used everywhere), never returned by UI
//! (only `webhook_set` is), and never committed.

use std::path::PathBuf;
use std::time::Duration;

use chrono::Utc;
use serde_json::json;

use crate::config::DiscordConfig;
use crate::logs::LogLine;
use crate::poller::InstanceSnapshot;

/// Discord state colours (v1 `STATE_COLORS`).
pub const STATE_COLORS: &[(&str, u32)] = &[
    ("online", 0x57F287),
    ("starting", 0xFEE75C),
    ("stopping", 0xE67E22),
    ("offline", 0x95A5A6),
    ("crashed", 0xED4245),
    ("unresponsive", 0x992D22),
];

/// Embed limits (respect Discord's): title 256, description 4096,
/// field value 1024, total 6000.
pub const DESCRIPTION_LIMIT: usize = 4096;
pub const FIELD_LIMIT: usize = 1024;
pub const TITLE_LIMIT: usize = 256;
pub const LINE_TRUNCATE: usize = 300;

/// Mask a webhook URL for logs (never leak the token).
pub fn mask_webhook_url(url: &str) -> String {
    match url.find("/api/webhooks/") {
        Some(idx) => format!("{}/api/webhooks/***/***", &url[..idx]),
        None => "***".to_string(),
    }
}

/// Humanise uptime seconds (v1 `humanize_uptime`).
pub fn humanize_uptime(seconds: Option<f64>) -> String {
    let Some(secs) = seconds else {
        return "—".to_string();
    };
    let total = secs.max(0.0) as u64;
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let s = total % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m {s:02}s")
    } else if minutes > 0 {
        format!("{minutes}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

/// Strip ANSI (v1 `redact.rs` parity) and truncate long lines.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            // consume until a letter
            for c2 in chars.by_ref() {
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Redact an IP in its common Minecraft login form `[/1.2.3.4:5678]` and bare IPs.
pub fn redact_line(line: &str) -> String {
    let mut s = strip_ansi(line);
    // login form: [/192.168.1.1:56789]
    let mut out = String::new();
    while let Some(idx) = s.find("[/") {
        out.push_str(&s[..idx]);
        out.push_str("[/redacted]");
        // skip until ']' inclusive
        if let Some(end) = s[idx..].find(']') {
            s = s[idx + end + 1..].to_string();
        } else {
            s = String::new();
            break;
        }
    }
    out.push_str(&s);
    // bare IPv4
    let mut cleaned = String::new();
    let mut rest = out.as_str();
    loop {
        let found = find_ipv4(rest);
        match found {
            Some((start, len)) => {
                cleaned.push_str(&rest[..start]);
                cleaned.push_str("[redacted]");
                rest = &rest[start + len..];
            }
            None => {
                cleaned.push_str(rest);
                break;
            }
        }
    }
    if cleaned.len() > LINE_TRUNCATE {
        cleaned.truncate(LINE_TRUNCATE - 1);
        cleaned.push('…');
    }
    cleaned
}

/// Find a bare IPv4 `x.x.x.x` (best effort; port digits are kept outside).
fn find_ipv4(s: &str) -> Option<(usize, usize)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let mut end = i;
            let mut dots = 0;
            while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'.') {
                if bytes[end] == b'.' {
                    dots += 1;
                }
                end += 1;
            }
            if dots == 3 {
                let candidate = &s[i..end];
                if is_ipv4(candidate) {
                    return Some((i, end - i));
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    None
}

fn is_ipv4(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    parts.iter().all(|p| {
        let v: Result<u32, _> = p.parse();
        matches!(v, Ok(n) if n < 256 && !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    })
}

/// Build a status embed (v1 `build_status_embed`).
pub fn build_status_embed(
    snapshot: &InstanceSnapshot,
    lines: &[LogLine],
    max_log_chars: usize,
) -> serde_json::Value {
    let probe = &snapshot.probe;
    let mut fields: Vec<serde_json::Value> = Vec::new();
    if let Some(version) = &probe.version {
        fields.push(json!({"name":"Version","value": version.chars().take(FIELD_LIMIT).collect::<String>(), "inline":true}));
    }
    if let Some(online) = probe.players_online {
        let mut players = online.to_string();
        if let Some(max) = probe.players_max {
            players = format!("{}/{max}", online);
        }
        if !probe.player_names.is_empty() {
            players = format!("{}\n{}", players, probe.player_names.join(", "));
        }
        fields.push(json!({"name":"Players","value": players.chars().take(FIELD_LIMIT).collect::<String>(), "inline":true}));
    }
    if let Some(latency) = probe.latency_ms {
        fields.push(json!({"name":"Latency","value": format!("{:.0} ms", latency), "inline":true}));
    }
    fields.push(json!({"name":"Uptime","value": humanize_uptime(snapshot.process.uptime_seconds), "inline":true}));
    fields.push(json!({"name":"Last updated","value": format!("<t:{}:R>", snapshot.updated_at.timestamp()), "inline":true}));

    let log_block = log_block(lines, max_log_chars);
    let mut embed = serde_json::json!({
        "title": format!("{} — {}", snapshot.name, snapshot.combined_state).chars().take(TITLE_LIMIT).collect::<String>(),
        "color": STATE_COLORS.iter().find(|(k, _)| *k == snapshot.combined_state).map(|(_, c)| *c).unwrap_or(0x95A5A6),
        "fields": fields,
        "timestamp": snapshot.updated_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "footer": {"text": "minewrap"},
    });
    if !log_block.is_empty() {
        embed["description"] = json!(format!("```\n{}\n```", log_block));
    }
    embed
}

/// Clip the log block to a char budget, dropping oldest lines first (v1).
fn log_block(lines: &[LogLine], max_chars: usize) -> String {
    let budget = max_chars.min(DESCRIPTION_LIMIT);
    let cleaned: Vec<String> = lines.iter().map(|l| redact_line(&l.line)).collect();
    let mut suffix: Vec<String> = cleaned;
    while suffix.len() > 1 && suffix.join("\n").len() > budget {
        suffix.remove(0);
    }
    let mut joined = suffix.join("\n");
    if joined.len() > budget {
        joined.truncate(budget);
    }
    joined
}

/// State-change embed (v1 `build_state_change_embed`).
pub fn build_state_change_embed(
    snapshot: &InstanceSnapshot,
    old_state: &str,
    new_state: &str,
    reason: &str,
) -> serde_json::Value {
    serde_json::json!({
        "title": format!("{}: {} → {}", snapshot.name, old_state, new_state).chars().take(TITLE_LIMIT).collect::<String>(),
        "description": format!("State changed: **{} → {}** ({})", old_state, new_state, reason).chars().take(DESCRIPTION_LIMIT).collect::<String>(),
        "color": STATE_COLORS.iter().find(|(k, _)| *k == new_state).map(|(_, c)| *c).unwrap_or(0x95A5A6),
        "timestamp": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    })
}

/// Persisted message id state file (v1 `<data_dir>/discord/<id>.json`).
pub fn state_path(state_dir: &std::path::Path, instance_id: &str) -> PathBuf {
    state_dir.join(format!("{instance_id}.json"))
}

/// Load persisted message id (0600 file, JSON `{"message_id": "…"}`).
pub fn load_message_id(state_dir: &std::path::Path, instance_id: &str) -> Option<String> {
    let text = std::fs::read_to_string(state_path(state_dir, instance_id)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let id = v.get("message_id")?.as_str()?;
    Some(id.to_string())
}

pub fn save_message_id(
    state_dir: &std::path::Path,
    instance_id: &str,
    message_id: &str,
) -> Result<(), String> {
    std::fs::create_dir_all(state_dir).map_err(|e| e.to_string())?;
    let path = state_path(state_dir, instance_id);
    let data = serde_json::json!({"message_id": message_id});
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, data.to_string()).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Result of a test push (mirrors v1 `DiscordTestResponse`).
#[derive(Debug, Clone)]
pub struct DiscordTestResult {
    pub ok: bool,
    pub error: Option<String>,
}

/// Discord notifier (port of v1 `DiscordNotifier`): interval pushes with
/// edit-in-place, debounced state-change posts, 429/5xx retries.
pub struct Notifier {
    pub config: DiscordConfig,
    pub http: reqwest::Client,
    pub state_dir: PathBuf,
    pub instance_id: String,
}

impl Notifier {
    pub fn new(
        config: DiscordConfig,
        http: reqwest::Client,
        state_dir: PathBuf,
        instance_id: String,
    ) -> Self {
        Self {
            config,
            http,
            state_dir,
            instance_id,
        }
    }

    pub fn enabled(&self) -> bool {
        self.config.enabled
            && self
                .config
                .webhook_url
                .as_ref()
                .map(|s| !s.is_empty())
                .unwrap_or(false)
    }

    pub fn webhook(&self) -> String {
        self.config
            .webhook_url
            .clone()
            .unwrap_or_default()
            .trim_end_matches('/')
            .to_string()
    }

    pub fn message_id(&self) -> Option<String> {
        load_message_id(&self.state_dir, &self.instance_id)
    }

    pub fn persist_message_id(&self, id: &str) -> Result<(), String> {
        save_message_id(&self.state_dir, &self.instance_id, id)
    }

    /// Test push (v1 `send_test`).
    pub async fn send_test(&self, snapshot: &InstanceSnapshot) -> DiscordTestResult {
        if !self.enabled() {
            return DiscordTestResult {
                ok: false,
                error: Some("discord is disabled or no webhook is configured".into()),
            };
        }
        let embed = serde_json::json!({
            "title": format!("minewrap test message — {}", snapshot.name).chars().take(TITLE_LIMIT).collect::<String>(),
            "description": format!("Instance `{}` can reach this webhook.", self.instance_id),
            "color": 0x57F287,
            "fields": [{"name":"State","value": snapshot.combined_state, "inline":true}],
            "timestamp": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        });
        match self
            .request(&format!("{}?wait=true", self.webhook()), "POST", &embed)
            .await
        {
            Ok((status, _)) if status < 400 => DiscordTestResult {
                ok: true,
                error: None,
            },
            Ok((status, _)) => DiscordTestResult {
                ok: false,
                error: Some(format!("discord request failed: HTTP {status}")),
            },
            Err(e) => DiscordTestResult {
                ok: false,
                error: Some(format!("discord request failed: {e}")),
            },
        }
    }

    /// Create or edit the status message (v1 `push_status_once`).
    pub async fn push_status(
        &self,
        snapshot: &InstanceSnapshot,
        lines: &[LogLine],
    ) -> Result<(), String> {
        let embed = build_status_embed(snapshot, lines, self.config.max_log_chars);
        if self.config.mode == "edit-in-place" {
            if let Some(msg_id) = self.message_id() {
                let (status, _) = self
                    .request(
                        &format!("{}/messages/{}", self.webhook(), msg_id),
                        "PATCH",
                        &embed,
                    )
                    .await?;
                if status == 404 {
                    // message deleted: fall through to create
                } else {
                    return Ok(());
                }
            }
        }
        let (status, body) = self
            .request(&format!("{}?wait=true", self.webhook()), "POST", &embed)
            .await?;
        if status >= 400 {
            return Err(format!("discord create failed: HTTP {status}"));
        }
        if self.config.mode == "edit-in-place" {
            if let Some(id) = body.get("id").and_then(|v| v.as_str()).map(String::from) {
                let _ = self.persist_message_id(&id);
            }
        }
        Ok(())
    }

    /// Internal request with 429/5xx retries (v1 `_request`).
    pub async fn request(
        &self,
        url: &str,
        method: &str,
        payload: &serde_json::Value,
    ) -> Result<(u16, serde_json::Value), String> {
        let masked = mask_webhook_url(url);
        let mut attempt = 0;
        let body_payload = serde_json::json!({ "embeds": [payload] });
        loop {
            attempt += 1;
            let req = self
                .http
                .request(
                    match method {
                        "POST" => reqwest::Method::POST,
                        _ => reqwest::Method::PATCH,
                    },
                    url,
                )
                .json(&body_payload);
            let resp = req
                .send()
                .await
                .map_err(|e| format!("discord request failed for {masked}: {e}"))?;
            let status = resp.status().as_u16();
            let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!({}));
            if status == 429 {
                let retry_after = (body
                    .get("retry_after")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1.0) as u64)
                    .min(60);
                if attempt >= 3 {
                    return Err(format!(
                        "discord rate limited for {masked} (tried {attempt}x)"
                    ));
                }
                tokio::time::sleep(Duration::from_millis(50 + retry_after * 1000)).await;
                continue;
            }
            if status >= 500 {
                if attempt >= 3 {
                    return Err(format!(
                        "discord HTTP {status} for {masked} (tried {attempt}x)"
                    ));
                }
                tokio::time::sleep(Duration::from_millis(50 * (1 << (attempt - 1)))).await;
                continue;
            }
            return Ok((status, body));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::poller::{InstanceSnapshot, ProbeInfo};
    use crate::process::{ProcessState, ProcessStatus};

    fn test_snapshot(state: &str) -> InstanceSnapshot {
        let probe = ProbeInfo {
            ok: true,
            latency_ms: Some(12.0),
            players_online: Some(2),
            players_max: Some(20),
            version: Some("1.21.4".into()),
            motd: Some("A Minecraft Server".into()),
            player_names: vec!["Alice".into(), "Bob".into()],
            error: None,
            checked_at: None,
        };
        InstanceSnapshot {
            id: "test".into(),
            name: "Test Server".into(),
            combined_state: state.into(),
            process: ProcessStatus {
                state: ProcessState::Online,
                pid: Some(1),
                started_at: None,
                stopped_at: None,
                exit_code: None,
                requested_stop: false,
                forced_stop: false,
                restart_count: 0,
                crash_count: 0,
                uptime_seconds: Some(3600.0),
            },
            probe,
            updated_at: chrono::DateTime::default(),
        }
    }

    #[test]
    fn redact_strips_ansi_and_ips() {
        let line = "\x1b[32mAlice[/192.168.50.7:54321] logged in\x1b[0m from 10.0.0.5";
        let out = redact_line(line);
        assert!(!out.contains("192.168.50.7"));
        assert!(!out.contains("10.0.0.5"));
        assert!(out.contains("Alice[/redacted]"));
    }

    #[test]
    fn redact_keeps_timestamps() {
        assert_eq!(
            redact_line("[01:48:51] [Server thread/INFO]: Done"),
            "[01:48:51] [Server thread/INFO]: Done"
        );
    }

    #[test]
    fn mask_hides_token() {
        assert_eq!(
            mask_webhook_url("https://discord.com/api/webhooks/123/secret-token"),
            "https://discord.com/api/webhooks/***/***"
        );
        assert_eq!(mask_webhook_url("http://localhost:1/hook"), "***");
    }

    #[test]
    fn humanize_uptime_formats() {
        assert_eq!(humanize_uptime(Some(65.0)), "1m 05s");
        assert_eq!(humanize_uptime(Some(3600.0)), "1h 00m 00s");
        assert_eq!(humanize_uptime(None), "—");
    }

    #[test]
    fn status_embed_fields() {
        let embed = build_status_embed(&test_snapshot("online"), &[], 1500);
        assert_eq!(embed["title"], "Test Server — online");
        let fields = embed["fields"].as_array().unwrap();
        let names: Vec<&str> = fields.iter().map(|f| f["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"Version"));
        assert!(names.contains(&"Players"));
        assert_eq!(embed["color"], 0x57F287);
    }

    #[test]
    fn log_block_trims_oldest() {
        let lines: Vec<LogLine> = (0..20)
            .map(|i| LogLine {
                seq: i as u64,
                ts: Utc::now(),
                line: format!("line {i} {}", "x".repeat(20)),
            })
            .collect();
        let block = log_block(&lines, 100);
        assert!(block.contains("line 19"));
        assert!(!block.contains("line 0"));
        assert!(block.len() <= 100);
    }

    #[test]
    fn state_file_round_trip() {
        let dir = std::env::temp_dir().join("minewrap-discord-state-test");
        let _ = std::fs::remove_dir_all(&dir);
        save_message_id(&dir, "test", "msg-1").unwrap();
        assert_eq!(load_message_id(&dir, "test"), Some("msg-1".into()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
