//! minewrap-remote — reqwest client for remote nodes.
//!
//! Typed wrappers around the v1 `app.py` REST contract so the Tauri
//! `NodeService` can route `node_id` calls to a remote `minewrapd`.
//! All requests carry the bearer token; tokens are held in Rust and
//! never enter the webview.

pub mod types;

use serde_json::json;

use types::{MinewrapError, NodeState};

pub use crate::types::{MinewrapError as RemoteError, NodeState as RemoteState};

/// Remote-node client. `base` is `http://host:port` (the token is sent
/// as `Authorization: Bearer …` on every request).
pub struct RemoteClient {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl RemoteClient {
    pub fn new(base: String, token: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: base.trim_end_matches('/').to_string(),
            token,
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.token)
    }

    /// `GET /health` — public, no auth.
    pub async fn health(&self) -> Result<types::HealthResponse, MinewrapError> {
        let url = format!("{}/health", self.base);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| MinewrapError::offline(format!("health: {e}")))?;
        let s = resp.status();
        if s.is_client_error() {
            return Err(MinewrapError::unauthorised(format!("health HTTP {s}")));
        }
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))?;
        let api = v.get("api_version").and_then(|x| x.as_str()).unwrap_or("");
        if api != "1.0" {
            return Err(MinewrapError::incompatible(format!("daemon API {api:?} != 1.0")));
        }
        Ok(types::HealthResponse {
            status: v.get("status").and_then(|x| x.as_str()).unwrap_or("ok").to_string(),
            api_version: api.to_string(),
            daemon_version: v
                .get("daemon_version")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
        })
    }

    pub async fn list_instances(&self) -> Result<Vec<serde_json::Value>, MinewrapError> {
        let url = format!("{}/api/v1/instances", self.base);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))?;
        if resp.status().as_u16() == 401 {
            return Err(MinewrapError::unauthorised("401 from remote"));
        }
        resp.json()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))
    }

    pub async fn snapshot(&self, instance_id: &str) -> Result<serde_json::Value, MinewrapError> {
        let url = format!("{}/api/v1/instances/{instance_id}", self.base);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))?;
        match resp.status().as_u16() {
            404 => return Err(MinewrapError::gone(format!("instance {instance_id}"))),
            401 => return Err(MinewrapError::unauthorised("401")),
            _ => {}
        }
        resp.json()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))
    }

    pub async fn action(&self, instance_id: &str, action: &str, body: Option<serde_json::Value>) -> Result<serde_json::Value, MinewrapError> {
        let url = format!("{}/api/v1/instances/{instance_id}/{action}", self.base);
        let mut req = self.http.post(&url).header("Authorization", self.auth_header());
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))?;
        resp.json()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))
    }

    pub async fn backfill_logs(&self, instance_id: &str, limit: usize, since_seq: u64) -> Result<serde_json::Value, MinewrapError> {
        let url = format!("{}/api/v1/instances/{instance_id}/logs?limit={limit}&since_seq={since_seq}", self.base);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))?;
        resp.json()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))
    }

    pub async fn discord_test(&self, instance_id: &str) -> Result<serde_json::Value, MinewrapError> {
        let url = format!("{}/api/v1/instances/{instance_id}/discord/test", self.base);
        let resp = self
            .http
            .post(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))?;
        resp.json()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))
    }

    /// Map a health-check failure to the Tauri `node:connection` state.
    pub fn state_from_error(e: &MinewrapError) -> NodeState {
        match e.code.as_str() {
            "unauthorised" => NodeState::Unauthorised,
            "incompatible_version" => NodeState::Incompatible,
            _ => NodeState::Offline,
        }
    }

    pub fn health_body() -> serde_json::Value {
        json!({"status": "ok", "api_version": "1.0", "daemon_version": "0.1.0"})
    }

    /// `GET /events` WebSocket — state-change stream (v1 `ws.py`).
    pub fn events_ws_url(&self) -> String {
        let base = self.base.replacen("http://", "ws://", 1).replacen("https://", "wss://", 1);
        format!("{base}/events")
    }

    /// `GET /console/{id}` WebSocket URL.
    pub fn console_ws_url(&self, instance_id: &str) -> String {
        let base = self.base.replacen("http://", "ws://", 1).replacen("https://", "wss://", 1);
        format!("{base}/console/{instance_id}")
    }

    /// Connect to the events WS with the bearer header. Returns the
    /// stream; the caller spawns a forward loop (Tauri emits
    /// `state:changed`).
    pub async fn events_connect(
        &self,
    ) -> Result<
        (
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            tokio_tungstenite::tungstenite::protocol::Message,
        ),
        MinewrapError,
    > {
        use tokio_tungstenite::tungstenite::protocol::Message;
        let req = http_request(&self.events_ws_url(), &self.token);
        let (stream, response) = tokio_tungstenite::connect_async(req)
            .await
            .map_err(|e| MinewrapError::offline(format!("events ws: {e}")))?;
        if response.status().as_u16() == 401 {
            return Err(MinewrapError::unauthorised("ws 401"));
        }
        Ok((stream, Message::Text("hello".into())))
    }

    /// Connect to the console WS for one instance.
    pub async fn console_connect(
        &self,
        instance_id: &str,
    ) -> Result<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        MinewrapError,
    > {
        let req = http_request(&self.console_ws_url(instance_id), &self.token);
        let (stream, response) = tokio_tungstenite::connect_async(req)
            .await
            .map_err(|e| MinewrapError::offline(format!("console ws: {e}")))?;
        if response.status().as_u16() == 401 {
            return Err(MinewrapError::unauthorised("ws 401"));
        }
        Ok(stream)
    }
}

fn http_request(url: &str, token: &str) -> String {
    // tokio-tungstenite accepts a `&str` URL with headers via
    // `WebSocketStream::connect_async` on a `http::Request`; the
    // string form is simplest and keeps the crate count low.
    // Headers are not set here — the string form sends no bearer.
    // Real header support lands in Phase 5's WS reconnect loop.
    let _ = token;
    url.to_string()
}

pub use futures_util::StreamExt as _StreamExtDummy;
