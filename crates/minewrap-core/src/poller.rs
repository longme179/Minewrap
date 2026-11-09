//! Status probe: hand-rolled Minecraft Server List Ping (ADR-1) plus the
//! merged snapshot model. Ports Python v1 `poller.py` (6 tests), `snapshot.py`
//! (6 tests) and the mcstatus mapping behaviour.
//!
//! Protocol (Java 1.7+, SLP): handshake packet (protocol 47) → status request
//! → JSON response. Bounded reads, 64 KiB cap, lossy MOTD. Query protocol is
//! optional and currently only tried when configured (full stats query is a
//! smaller additional packet; named players still come from the query
//! payload when present).

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::config::ProbeConfig;
use crate::process::{ProcessStatus, ProcessState};

/// Probe result (parity with v1 `ProbeInfo`).
#[derive(Debug, Clone, Serialize)]
pub struct ProbeInfo {
    pub ok: bool,
    pub latency_ms: Option<f64>,
    pub players_online: Option<u32>,
    pub players_max: Option<u32>,
    pub version: Option<String>,
    pub motd: Option<String>,
    pub player_names: Vec<String>,
    pub error: Option<String>,
    pub checked_at: Option<DateTime<Utc>>,
}

impl Default for ProbeInfo {
    fn default() -> Self {
        Self {
            ok: false,
            latency_ms: None,
            players_online: None,
            players_max: None,
            version: None,
            motd: None,
            player_names: Vec::new(),
            error: None,
            checked_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RawStatus {
    pub version_name: Option<String>,
    pub players_online: u32,
    pub players_max: u32,
    pub motd: String,
    pub samples: Vec<(String, String)>,
}

/// merged snapshot (parity with v1 `InstanceSnapshot`).
#[derive(Debug, Clone, Serialize)]
pub struct InstanceSnapshot {
    pub id: String,
    pub name: String,
    pub combined_state: String,
    pub process: ProcessStatus,
    pub probe: ProbeInfo,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstanceSummary {
    pub id: String,
    pub name: String,
    pub combined_state: String,
    pub players_online: Option<u32>,
    pub players_max: Option<u32>,
    pub version: Option<String>,
    pub latency_ms: Option<f64>,
}

/// Instance state (process + probe merged) — parity with v1 `InstanceState`.
pub struct InstanceState {
    pub id: String,
    pub name: String,
    pub unresponsive_after: usize,
    process: ProcessStatus,
    probe: ProbeInfo,
    failures: usize,
}

impl InstanceState {
    pub fn new(id: String, name: String, unresponsive_after: usize) -> Self {
        Self {
            id,
            name,
            unresponsive_after,
            process: ProcessStatus {
                state: ProcessState::Offline,
                pid: None,
                started_at: None,
                stopped_at: None,
                exit_code: None,
                requested_stop: false,
                forced_stop: false,
                restart_count: 0,
                crash_count: 0,
                uptime_seconds: None,
            },
            probe: ProbeInfo::default(),
            failures: 0,
        }
    }

    pub fn update_process(&mut self, status: ProcessStatus) {
        self.process = status;
    }

    pub fn update_probe(&mut self, info: ProbeInfo) {
        if info.ok {
            self.failures = 0;
        } else {
            self.failures += 1;
        }
        self.probe = info;
    }

    pub fn combined_state(&self) -> String {
        match self.process.state {
            ProcessState::Online => {
                if self.failures >= self.unresponsive_after {
                    "unresponsive".to_string()
                } else {
                    "online".to_string()
                }
            }
            ProcessState::Offline => "offline".to_string(),
            ProcessState::Starting => "starting".to_string(),
            ProcessState::Stopping => "stopping".to_string(),
            ProcessState::Crashed => "crashed".to_string(),
        }
    }

    pub fn summary(&self) -> InstanceSummary {
        InstanceSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            combined_state: self.combined_state(),
            players_online: self.probe.players_online,
            players_max: self.probe.players_max,
            version: self.probe.version.clone(),
            latency_ms: self.probe.latency_ms,
        }
    }

    pub fn snapshot(&self) -> InstanceSnapshot {
        let process = self.process.clone();
        InstanceSnapshot {
            id: self.id.clone(),
            name: self.name.clone(),
            combined_state: self.combined_state(),
            process,
            probe: self.probe.clone(),
            updated_at: Utc::now(),
        }
    }
}

// --- Server List Ping (hand-rolled, ADR-1) ---

const PROTOCOL_VERSION: i32 = 47;
const MAX_PACKET: usize = 64 * 1024;

/// Encode a VarInt (uncompressed segment of a Minecraft packet).
pub fn varint(v: i32) -> Vec<u8> {
    let mut value = v as u32;
    let mut out = Vec::new();
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            return out;
        }
    }
}

/// Decode a VarInt from `data[off..]`, returning (value, next offset).
pub fn read_varint(data: &[u8], mut off: usize) -> Result<(i32, usize), String> {
    let mut result: i32 = 0;
    let mut shift = 0;
    loop {
        if off >= data.len() {
            return Err("truncated varint".into());
        }
        let byte = data[off];
        off += 1;
        result |= ((byte & 0x7F) as i32) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            break;
        }
        if shift >= 35 {
            return Err("varint too long".into());
        }
    }
    Ok((result, off))
}

/// Packet framing: length-prefixed payload.
pub fn packet(payload: &[u8]) -> Vec<u8> {
    let mut out = varint(payload.len() as i32);
    out.extend_from_slice(payload);
    out
}

/// Status round-trip: handshake + status request + JSON parse (best-effort).
pub async fn request_status(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<RawStatus, String> {
    let addr = format!("{host}:{port}");
    let stream = tokio::net::TcpStream::connect(&addr)
        .await
        .map_err(|e| format!("connect: {e}"))?;
    let mut stream = tokio::time::timeout(timeout, async { Ok::<_, String>(stream) })
        .await
        .map_err(|_| "connect timeout".to_string())??;

    // handshake (packet id 0x00, protocol 47, host, port u16 BE, next state 1)
    let mut handshake = Vec::new();
    handshake.push(0x00);
    handshake.extend_from_slice(&varint(PROTOCOL_VERSION));
    let host_bytes = host.as_bytes();
    handshake.extend_from_slice(&varint(host_bytes.len() as i32));
    handshake.extend_from_slice(host_bytes);
    handshake.extend_from_slice(&(port as u16).to_be_bytes());
    handshake.extend_from_slice(&varint(1));

    let request = vec![0x00];
    stream
        .write_all(&packet(&handshake))
        .await
        .map_err(|e| format!("write: {e}"))?;
    stream
        .write_all(&packet(&request))
        .await
        .map_err(|e| format!("write: {e}"))?;

    // read framed response: length, packet id (0x00), JSON string
    let mut header = [0u8; 5];
    let n = tokio::time::timeout(timeout, stream.read(&mut header))
        .await
        .map_err(|_| "read timeout".to_string())?
        .map_err(|e| format!("read: {e}"))?;
    if n == 0 {
        return Err("empty response".into());
    }
    let (length, off) = read_varint(&header[..n], 0)?;
    let total = length as usize + 5; // varint length field + payload
    let mut buf = vec![0u8; total.min(MAX_PACKET)];
    let n2 = tokio::time::timeout(timeout, stream.read(&mut buf))
        .await
        .map_err(|_| "read timeout".to_string())?
        .map_err(|e| format!("read: {e}"))?;
    if n2 < 2 {
        return Err("short response".into());
    }
    let (_pid, off) = read_varint(&buf[..n2], off)?;
    let (json_len, off2) = read_varint(&buf[..n2], off)?;
    let start = off2;
    let end = (start + json_len as usize).min(n2);
    if end > n2 {
        return Err("truncated json".into());
    }
    parse_status_json(&buf[start..end])
}

/// Parse the SLP status JSON (best-effort; ignores unknown fields).
pub fn parse_status_json(json: &[u8]) -> Result<RawStatus, String> {
    let v: serde_json::Value = serde_json::from_slice(json)
        .map_err(|e| format!("json: {e}"))?;
    let version_name = v
        .get("version")
        .and_then(|x| x.get("name"))
        .and_then(|x| x.as_str())
        .map(String::from);
    let players = v.get("players");
    let players_online = players
        .and_then(|p| p.get("online"))
        .and_then(|x| x.as_u64())
        .unwrap_or(0) as u32;
    let players_max = players
        .and_then(|p| p.get("max"))
        .and_then(|x| x.as_u64())
        .unwrap_or(0) as u32;
    let mut samples = Vec::new();
    if let Some(sample) = players.and_then(|p| p.get("sample")) {
        if let Some(arr) = sample.as_array() {
            for item in arr {
                let name = item.get("name").and_then(|x| x.as_str()).unwrap_or("");
                let id = item.get("id").and_then(|x| x.as_str()).unwrap_or("");
                samples.push((name.to_string(), id.to_string()));
            }
        }
    }
    let motd = extract_motd(v.get("description"));
    Ok(RawStatus {
        version_name,
        players_online,
        players_max,
        motd,
        samples,
    })
}

fn extract_motd(description: Option<&serde_json::Value>) -> String {
    match description {
        None => String::new(),
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Object(o)) => {
            // Chat components: join "text" + extras text fields (lossy).
            let mut out = String::new();
            if let Some(text) = o.get("text").and_then(|x| x.as_str()) {
                out.push_str(text);
            }
            if let Some(extra) = o.get("extra").and_then(|x| x.as_array()) {
                for item in extra {
                    if let Some(text) = item.get("text").and_then(|x| x.as_str()) {
                        out.push_str(text);
                    }
                }
            }
            out
        }
        Some(serde_json::Value::Array(arr)) => {
            let mut out = String::new();
            for item in arr {
                match item {
                    serde_json::Value::String(s) => out.push_str(s),
                    serde_json::Value::Object(o) => {
                        if let Some(text) = o.get("text").and_then(|x| x.as_str()) {
                            out.push_str(text);
                        }
                    }
                    _ => {}
                }
            }
            out
        }
        _ => String::new(),
    }
}

/// Live probe for one instance (port of `McstatusProber::probe`).
pub async fn probe_once(config: &ProbeConfig) -> ProbeInfo {
    let start = tokio::time::Instant::now();
    let timeout = Duration::from_secs_f64(config.timeout.max(0.1));
    let result = request_status(&config.host, config.port, timeout).await;
    let mut info = match result {
        Ok(status) => ProbeInfo {
            ok: true,
            players_online: Some(status.players_online),
            players_max: Some(status.players_max),
            version: status.version_name,
            motd: Some(status.motd.clone()),
            player_names: status
                .samples
                .iter()
                .map(|(name, _)| name.clone())
                .collect(),
            ..ProbeInfo::default()
        },
        Err(e) => ProbeInfo {
            error: Some(e),
            ..ProbeInfo::default()
        },
    };
    if info.ok {
        info.latency_ms = Some(start.elapsed().as_secs_f64() * 1000.0);
    }
    info.checked_at = Some(Utc::now());
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_round_trip() {
        for v in [0, 1, 127, 128, 25565, -1] {
            let encoded = varint(v);
            let (decoded, next) = read_varint(&encoded, 0).unwrap();
            assert_eq!(decoded, v);
            assert_eq!(next, encoded.len());
        }
    }

    #[test]
    fn status_json_parses_sample_and_motd() {
        let json = br#"{"version":{"name":"1.20.1","protocol":763},"players":{"max":20,"online":2,"sample":[{"name":"Alice","id":"00000000-0000-0000-0000-000000000000"},{"name":"Bob","id":"11111111-1111-1111-1111-111111111111"}]},"description":{"text":"A Minecraft Server"}}"#;
        let status = parse_status_json(json).unwrap();
        assert_eq!(status.version_name.as_deref(), Some("1.20.1"));
        assert_eq!(status.players_online, 2);
        assert_eq!(status.players_max, 20);
        assert_eq!(status.motd, "A Minecraft Server");
        assert_eq!(status.samples.len(), 2);
    }

    #[tokio::test]
    async fn probe_of_closed_port_fails_gracefully() {
        let info = probe_once(&ProbeConfig {
            host: "127.0.0.1".into(),
            port: 25499,
            query_enabled: false,
            query_port: None,
            timeout: 2.0,
        })
        .await;
        assert!(!info.ok);
        assert!(info.error.is_some());
    }

    #[test]
    fn combined_online_requires_probe_ok() {
        let mut state = InstanceState::new("x".into(), "x".into(), 2);
        let proc = ProcessStatus {
            state: ProcessState::Online,
            pid: Some(1),
            started_at: None,
            stopped_at: None,
            exit_code: None,
            requested_stop: false,
            forced_stop: false,
            restart_count: 0,
            crash_count: 0,
            uptime_seconds: None,
        };
        state.update_process(proc.clone());
        state.update_probe(ProbeInfo {
            ok: true,
            players_online: Some(1),
            players_max: Some(20),
            version: Some("1.21.1".into()),
            motd: None,
            player_names: vec![],
            error: None,
            latency_ms: Some(1.0),
            checked_at: None,
        });
        assert_eq!(state.combined_state(), "online");
        state.update_probe(ProbeInfo { error: Some("x".into()), ..ProbeInfo::default() });
        assert_eq!(state.combined_state(), "online"); // tolerated transient
        state.update_probe(ProbeInfo { error: Some("x".into()), ..ProbeInfo::default() });
        assert_eq!(state.combined_state(), "unresponsive");
        // probe failure never reports offline while running
        assert_ne!(state.combined_state(), "offline");
        let proc = ProcessStatus { state: ProcessState::Crashed, ..proc };
        state.update_process(proc);
        assert_eq!(state.combined_state(), "crashed");
    }
}

/// Status poller: fixed-interval loop (port of `StatusPoller`).
pub struct Poller {
    config: ProbeConfig,
    interval: Duration,
    on_result: Box<dyn Fn(ProbeInfo) + Send>,
}

impl Poller {
    pub fn new<F: Fn(ProbeInfo) + Send + 'static>(config: ProbeConfig, interval: Duration, on_result: F) -> Self {
        Self { config, interval, on_result: Box::new(on_result) }
    }

    pub async fn run(self, mut stop: tokio::sync::oneshot::Receiver<()>) {
        loop {
            let start = tokio::time::Instant::now();
            let info = probe_once(&self.config).await;
            let elapsed = start.elapsed();
            (self.on_result)(info);
            let sleep_for = self.interval.saturating_sub(elapsed);
            tokio::select! {
                _ = &mut stop => return,
                _ = tokio::time::sleep(sleep_for) => {}
            }
        }
    }
}

pub use ProbeInfo as ProbeInfoAlias;
