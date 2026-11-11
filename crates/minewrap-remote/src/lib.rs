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
#[derive(Clone)]
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
            return Err(MinewrapError::incompatible(format!(
                "daemon API {api:?} != 1.0"
            )));
        }
        Ok(types::HealthResponse {
            status: v
                .get("status")
                .and_then(|x| x.as_str())
                .unwrap_or("ok")
                .to_string(),
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

    pub async fn action(
        &self,
        instance_id: &str,
        action: &str,
        body: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, MinewrapError> {
        let url = format!("{}/api/v1/instances/{instance_id}/{action}", self.base);
        let mut req = self
            .http
            .post(&url)
            .header("Authorization", self.auth_header());
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

    /// `GET /api/v1/instances` — typed.
    pub async fn list_instances_typed(
        &self,
    ) -> Result<Vec<types::InstanceSummaryDto>, MinewrapError> {
        let url = format!("{}/api/v1/instances", self.base);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))?;
        self.json_typed(resp).await
    }

    /// `GET /api/v1/instances/{id}` — typed snapshot.
    pub async fn snapshot_typed(
        &self,
        instance_id: &str,
    ) -> Result<types::SnapshotDto, MinewrapError> {
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
        let raw: types::DaemonSnapshot = self.json_typed(resp).await?;
        Ok(raw.into())
    }

    /// `POST /api/v1/instances/{id}/command`.
    pub async fn send_command(
        &self,
        instance_id: &str,
        command: &str,
    ) -> Result<serde_json::Value, MinewrapError> {
        self.action(instance_id, "command", Some(json!({"command": command})))
            .await
    }

    /// `GET …/logs?limit&since_seq` — typed lines.
    pub async fn logs_typed(
        &self,
        instance_id: &str,
        limit: usize,
        since_seq: u64,
    ) -> Result<Vec<types::LogLineDto>, MinewrapError> {
        #[derive(serde::Deserialize)]
        struct LogsBody {
            #[serde(default)]
            lines: Vec<types::LogLineDto>,
        }
        let url = format!(
            "{}/api/v1/instances/{instance_id}/logs?limit={limit}&since_seq={since_seq}",
            self.base
        );
        let resp = self
            .http
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))?;
        let body: LogsBody = self.json_typed(resp).await?;
        Ok(body.lines)
    }

    /// `GET …/config` — masked view as raw JSON.
    pub async fn get_config(&self, instance_id: &str) -> Result<serde_json::Value, MinewrapError> {
        let url = format!("{}/api/v1/instances/{instance_id}/config", self.base);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))?;
        self.json_typed(resp).await
    }

    /// `PUT …/config` — write-only patch.
    pub async fn update_config(
        &self,
        instance_id: &str,
        patch: serde_json::Value,
    ) -> Result<types::Action, MinewrapError> {
        let url = format!("{}/api/v1/instances/{instance_id}/config", self.base);
        let resp = self
            .http
            .put(&url)
            .header("Authorization", self.auth_header())
            .json(&patch)
            .send()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))?;
        self.json_typed(resp).await
    }

    /// `POST /api/v1/instances` (v1.10.0).
    pub async fn add_instance(
        &self,
        config: serde_json::Value,
    ) -> Result<types::Action, MinewrapError> {
        let url = format!("{}/api/v1/instances", self.base);
        let resp = self
            .http
            .post(&url)
            .header("Authorization", self.auth_header())
            .json(&config)
            .send()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))?;
        if resp.status().as_u16() == 409 {
            let detail = self.error_detail(resp).await;
            return Err(MinewrapError::invalid(
                detail.unwrap_or_else(|| "duplicate instance id".into()),
            ));
        }
        self.json_typed(resp).await
    }

    /// `DELETE /api/v1/instances/{id}` (v1.10.0).
    pub async fn remove_instance(&self, instance_id: &str) -> Result<types::Action, MinewrapError> {
        let url = format!("{}/api/v1/instances/{instance_id}", self.base);
        let resp = self
            .http
            .delete(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))?;
        match resp.status().as_u16() {
            404 => return Err(MinewrapError::gone(format!("instance {instance_id}"))),
            401 => return Err(MinewrapError::unauthorised("401")),
            _ => {}
        }
        self.json_typed(resp).await
    }

    /// `GET /api/v1/java` (v1.10.0) — detection on the daemon host.
    pub async fn detect_java(&self) -> Result<Vec<types::JavaRuntimeDto>, MinewrapError> {
        let url = format!("{}/api/v1/java", self.base);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))?;
        self.json_typed(resp).await
    }

    /// `POST /api/v1/scan-workdir` (v1.10.0).
    pub async fn scan_workdir(&self, work_dir: &str) -> Result<serde_json::Value, MinewrapError> {
        let url = format!("{}/api/v1/scan-workdir", self.base);
        let resp = self
            .http
            .post(&url)
            .header("Authorization", self.auth_header())
            .json(&json!({"work_dir": work_dir}))
            .send()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))?;
        self.json_typed(resp).await
    }

    /// `GET/PUT …/properties`.
    pub async fn read_properties(
        &self,
        instance_id: &str,
    ) -> Result<types::PropertiesDoc, MinewrapError> {
        let url = format!("{}/api/v1/instances/{instance_id}/properties", self.base);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))?;
        match resp.status().as_u16() {
            404 => return Err(MinewrapError::gone(format!("server.properties not found"))),
            401 => return Err(MinewrapError::unauthorised("401")),
            _ => {}
        }
        self.json_typed(resp).await
    }

    pub async fn write_properties(
        &self,
        instance_id: &str,
        text: &str,
    ) -> Result<types::Action, MinewrapError> {
        let url = format!("{}/api/v1/instances/{instance_id}/properties", self.base);
        let resp = self
            .http
            .put(&url)
            .header("Authorization", self.auth_header())
            .json(&json!({"text": text}))
            .send()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))?;
        self.json_typed(resp).await
    }

    /// `GET …/mods` + `POST …/mods/toggle`.
    pub async fn list_mods(&self, instance_id: &str) -> Result<Vec<types::ModDto>, MinewrapError> {
        let url = format!("{}/api/v1/instances/{instance_id}/mods", self.base);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))?;
        self.json_typed(resp).await
    }

    pub async fn toggle_mod(
        &self,
        instance_id: &str,
        name: &str,
    ) -> Result<types::Action, MinewrapError> {
        let url = format!("{}/api/v1/instances/{instance_id}/mods/toggle", self.base);
        let resp = self
            .http
            .post(&url)
            .header("Authorization", self.auth_header())
            .json(&json!({"name": name}))
            .send()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))?;
        self.json_typed(resp).await
    }

    /// `POST …/discord/test` — typed.
    pub async fn discord_test(
        &self,
        instance_id: &str,
    ) -> Result<types::DiscordTestResponse, MinewrapError> {
        let url = format!("{}/api/v1/instances/{instance_id}/discord/test", self.base);
        let resp = self
            .http
            .post(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| MinewrapError::offline(e.to_string()))?;
        self.json_typed(resp).await
    }

    /// Health with a connect/response timeout (used by the GUI's
    /// auto-spawn probe).
    pub async fn health_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Result<types::HealthResponse, MinewrapError> {
        match tokio::time::timeout(timeout, self.health()).await {
            Ok(r) => r,
            Err(_) => Err(MinewrapError::offline("health check timeout")),
        }
    }

    /// Read a response into a typed value, mapping HTTP error codes to
    /// the shared error envelope.
    async fn json_typed<T: serde::de::DeserializeOwned>(
        &self,
        resp: reqwest::Response,
    ) -> Result<T, MinewrapError> {
        let status = resp.status().as_u16();
        if status == 401 {
            return Err(MinewrapError::unauthorised("401 from remote"));
        }
        if status == 404 {
            return Err(MinewrapError::gone("404 from remote"));
        }
        if status >= 400 {
            let detail = self
                .error_detail(resp)
                .await
                .unwrap_or_else(|| format!("HTTP {status}"));
            return Err(MinewrapError::invalid(detail));
        }
        resp.json()
            .await
            .map_err(|e| MinewrapError::offline(format!("decode: {e}")))
    }

    async fn error_detail(&self, resp: reqwest::Response) -> Option<String> {
        let v: serde_json::Value = resp.json().await.ok()?;
        v.get("detail")
            .and_then(|d| d.as_str())
            .map(String::from)
            .or_else(|| Some(v.to_string()))
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

    /// `GET /events` WebSocket URL.
    pub fn events_ws_url(&self) -> String {
        let base = self
            .base
            .replacen("http://", "ws://", 1)
            .replacen("https://", "wss://", 1);
        format!("{base}/events")
    }

    /// `GET /console/{id}` WebSocket URL.
    pub fn console_ws_url(&self, instance_id: &str) -> String {
        let base = self
            .base
            .replacen("http://", "ws://", 1)
            .replacen("https://", "wss://", 1);
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
        let req = self.ws_request(&self.events_ws_url());
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
        let req = self.ws_request(&self.console_ws_url(instance_id));
        let (stream, response) = tokio_tungstenite::connect_async(req)
            .await
            .map_err(|e| MinewrapError::offline(format!("console ws: {e}")))?;
        if response.status().as_u16() == 401 {
            return Err(MinewrapError::unauthorised("ws 401"));
        }
        Ok(stream)
    }

    /// Build a handshake request carrying the bearer token. v1.9.3
    /// passed a bare URL (no headers — the daemon would 401); the
    /// request now carries the full WS handshake header set.
    fn ws_request(&self, url: &str) -> tokio_tungstenite::tungstenite::http::Request<()> {
        use tokio_tungstenite::tungstenite::handshake::client::generate_key;
        use tokio_tungstenite::tungstenite::http::Request;
        let host = self
            .base
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .to_string();
        Request::builder()
            .uri(url)
            .header("Host", &host)
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", generate_key())
            .header("Authorization", self.auth_header())
            .body(())
            .expect("ws request")
    }
}
