//! REST API — the v1 `app.py` contract: `/health` (public), bearer
//! auth, `GET /instances`, snapshot, start/stop/restart, command, logs,
//! config get/put, discord test, `openapi.json`.
//!
//! v1.10.0 (P0 daemon-first) additions — all additive, API stays "1.0":
//! `POST /instances`, `DELETE /instances/{id}`, `GET /java`,
//! `POST /scan-workdir`; the `/events` WebSocket now also forwards
//! `LogLine` (batched, seq-tagged) and `RestartLoop`.
//!
//! Route shapes are the v1 FastAPI shapes (ported field-for-field in
//! PARITY.md §6) with api version **1.0** (v1 was "0.1").

use std::sync::Arc;

use axum::{
    Json, Router,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;

use crate::auth::TokenAuth;
use crate::instances::InstanceManager;

pub const API_VERSION: &str = minewrap_core::API_VERSION;

/// Build the daemon router with the bearer auth middleware applied
/// to every route except `/health`.
///
/// The token lives in the keyring/`0600` store in the GUI; the daemon
/// receives it via `MINEWRAP_TOKEN` (for headless use) or the
/// `data/token` file generated at startup (v1 parity).
pub fn create_router(manager: Arc<InstanceManager>, auth: TokenAuth) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/openapi.json", get(openapi))
        .route(
            "/api/v1/instances",
            get(list_instances).post(create_instance),
        )
        .route(
            "/api/v1/instances/{id}",
            get(get_snapshot).delete(delete_instance),
        )
        .route("/api/v1/instances/{id}/start", post(instance_start))
        .route("/api/v1/instances/{id}/stop", post(instance_stop))
        .route("/api/v1/instances/{id}/restart", post(instance_restart))
        .route("/api/v1/instances/{id}/command", post(instance_command))
        .route("/api/v1/instances/{id}/logs", get(instance_logs))
        .route(
            "/api/v1/instances/{id}/config",
            get(get_config).put(update_config),
        )
        .route("/api/v1/instances/{id}/discord/test", post(discord_test))
        .route(
            "/api/v1/instances/{id}/properties",
            get(read_properties).put(write_properties),
        )
        .route("/api/v1/instances/{id}/mods", get(list_mods))
        .route("/api/v1/instances/{id}/mods/toggle", post(toggle_mod))
        .route("/api/v1/java", get(detect_java))
        .route("/api/v1/scan-workdir", post(scan_workdir))
        .route("/console/{id}", get(ws::console_ws))
        .route("/events", get(ws::events_ws))
        .layer(axum::extract::Extension((manager, auth)))
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({"detail": "invalid or missing bearer token"})),
    )
        .into_response()
}

fn check_auth(auth: &TokenAuth, headers: &HeaderMap) -> Result<(), Response> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| crate::auth::bearer_from_header(h))
        .unwrap_or_default();
    if auth.verify(&token) {
        Ok(())
    } else {
        Err(unauthorized())
    }
}

// `health` is public (v1 parity).
async fn health() -> impl axum::response::IntoResponse {
    Json(json!({
        "status": "ok",
        "api_version": API_VERSION,
        "daemon_version": minewrap_core::CORE_VERSION,
    }))
}

async fn openapi() -> impl axum::response::IntoResponse {
    // Generated schema sketch; Axum emits it via the `utoipa` feature in
    // a later phase (v1 shipped a committed `openapi.json`).
    Json(json!({
        "openapi": "3.0.0",
        "info": {"title": "minewrapd", "version": "1.0"},
        "paths": {
            "/health": {"get": {"responses": {"200": {"description": "ok"}}}},
        },
    }))
}

async fn list_instances(
    axum::extract::Extension((mgr, auth)): axum::extract::Extension<(
        Arc<InstanceManager>,
        TokenAuth,
    )>,
    headers: HeaderMap,
) -> impl axum::response::IntoResponse {
    if let Err(e) = check_auth(&auth, &headers) {
        return e.into_response();
    }
    let summaries: Vec<_> = mgr
        .ids()
        .into_iter()
        .filter_map(|id| mgr.summary_of(&id))
        .collect();
    Json(json!(summaries)).into_response()
}

async fn get_snapshot(
    axum::extract::Extension((mgr, auth)): axum::extract::Extension<(
        Arc<InstanceManager>,
        TokenAuth,
    )>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    if let Err(e) = check_auth(&auth, &headers) {
        return e.into_response();
    }
    match mgr.snapshot_json(&id) {
        Some(snap) => Json(snap).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": format!("unknown instance: {id}")})),
        )
            .into_response(),
    }
}

/// `POST /api/v1/instances` — create an instance (v1.10.0, task B4).
async fn create_instance(
    axum::extract::Extension((mgr, auth)): axum::extract::Extension<(
        Arc<InstanceManager>,
        TokenAuth,
    )>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> impl axum::response::IntoResponse {
    if let Err(e) = check_auth(&auth, &headers) {
        return e.into_response();
    }
    let cfg: minewrap_core::InstanceConfig = match serde_json::from_value(body) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"detail": format!("invalid instance config: {e}")})),
            )
                .into_response();
        }
    };
    if cfg.id.trim().is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": "instance id must not be empty"})),
        )
            .into_response();
    }
    if mgr.config_of(&cfg.id).is_some() {
        return (
            StatusCode::CONFLICT,
            Json(json!({"detail": format!("duplicate instance id: {}", cfg.id)})),
        )
            .into_response();
    }
    match mgr.add_instance(cfg.clone()) {
        Ok(()) => (
            StatusCode::CREATED,
            Json(json!({"id": cfg.id, "name": cfg.display_name()})),
        )
            .into_response(),
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"detail": e}))).into_response(),
    }
}

/// `DELETE /api/v1/instances/{id}` — stop gracefully (stdin `stop` →
/// SIGTERM → SIGKILL via the process manager), then remove and persist
/// (v1.10.0, task B4).
async fn delete_instance(
    axum::extract::Extension((mgr, auth)): axum::extract::Extension<(
        Arc<InstanceManager>,
        TokenAuth,
    )>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    use minewrap_core::ProcessState;
    if let Err(e) = check_auth(&auth, &headers) {
        return e.into_response();
    }
    let (stop_timeout, kill_grace) = {
        let daemon = mgr.daemon.lock().unwrap_or_else(|e| e.into_inner());
        (
            daemon.daemon.defaults.stop_timeout,
            daemon.daemon.defaults.kill_grace_seconds,
        )
    };
    if mgr.config_of(&id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": format!("unknown instance: {id}")})),
        )
            .into_response();
    }
    // Graceful stop first; watch the merged state until it settles.
    mgr.send_control(&id, minewrap_core::Control::Stop);
    let deadline = tokio::time::Instant::now()
        + std::time::Duration::from_secs_f64(stop_timeout + kill_grace + 2.0);
    loop {
        let settled = matches!(
            mgr.snapshot_of(&id).map(|s| s.process.state),
            None | Some(ProcessState::Offline) | Some(ProcessState::Crashed)
        );
        if settled || tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    // Force-shutdown any survivor (kills the process group).
    mgr.send_control(&id, minewrap_core::Control::Shutdown);
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    if !mgr.finish_remove_instance(&id) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": format!("unknown instance: {id}")})),
        )
            .into_response();
    }
    Json(json!({"ok": true})).into_response()
}

/// `GET /api/v1/java` — Java detection runs on the daemon host (the
/// machine the server runs on), v1.10.0 task B4.
async fn detect_java(
    axum::extract::Extension((_mgr, auth)): axum::extract::Extension<(
        Arc<InstanceManager>,
        TokenAuth,
    )>,
    headers: HeaderMap,
) -> impl axum::response::IntoResponse {
    if let Err(e) = check_auth(&auth, &headers) {
        return e.into_response();
    }
    // Detection is a filesystem scan; run it on the blocking pool.
    let runtimes = tokio::task::spawn_blocking(minewrap_core::detect_all)
        .await
        .unwrap_or_default();
    Json(json!(runtimes)).into_response()
}

/// `POST /api/v1/scan-workdir` — launch-candidate scan + contents
/// report, same confinement as the v1.9.3 GUI command (v1.10.0 B4).
async fn scan_workdir(
    axum::extract::Extension((_mgr, auth)): axum::extract::Extension<(
        Arc<InstanceManager>,
        TokenAuth,
    )>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> impl axum::response::IntoResponse {
    if let Err(e) = check_auth(&auth, &headers) {
        return e.into_response();
    }
    let work_dir = body
        .get("work_dir")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let dir = std::path::PathBuf::from(&work_dir);
    let dir = if dir.is_absolute() {
        dir
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(dir),
            Err(e) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({"detail": e.to_string()})),
                )
                    .into_response();
            }
        }
    };
    if !dir.exists() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": format!("{} does not exist", dir.to_string_lossy())})),
        )
            .into_response();
    }
    let result = tokio::task::spawn_blocking(move || {
        let candidates = minewrap_core::scan_work_dir(&dir);
        let report = minewrap_core::report_work_dir(&dir);
        json!({
            "candidates": candidates.iter().map(|c| json!({
                "path": c.path.to_string_lossy(),
                "kind": c.kind,
                "server_type": c.server_type,
                "mc_version": c.mc_version,
                "suggested_launch_args": c.suggested_launch_args,
                "min_java": c.min_java,
            })).collect::<Vec<_>>(),
            "report": report,
        })
    })
    .await
    .unwrap_or_else(|_| json!({"candidates": [], "report": null}));
    Json(result).into_response()
}

async fn instance_start(
    axum::extract::Extension((mgr, auth)): axum::extract::Extension<(
        Arc<InstanceManager>,
        TokenAuth,
    )>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    if let Err(e) = check_auth(&auth, &headers) {
        return e.into_response();
    }
    if mgr.config_of(&id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": format!("unknown instance: {id}")})),
        )
            .into_response();
    }
    let ok = mgr.send_control(&id, minewrap_core::Control::Start);
    Json(json!({"ok": ok})).into_response()
}

async fn instance_stop(
    axum::extract::Extension((mgr, auth)): axum::extract::Extension<(
        Arc<InstanceManager>,
        TokenAuth,
    )>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    if let Err(e) = check_auth(&auth, &headers) {
        return e.into_response();
    }
    if mgr.config_of(&id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": format!("unknown instance: {id}")})),
        )
            .into_response();
    }
    let ok = mgr.send_control(&id, minewrap_core::Control::Stop);
    Json(json!({"ok": ok})).into_response()
}

async fn instance_restart(
    axum::extract::Extension((mgr, auth)): axum::extract::Extension<(
        Arc<InstanceManager>,
        TokenAuth,
    )>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    if let Err(e) = check_auth(&auth, &headers) {
        return e.into_response();
    }
    if mgr.config_of(&id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": format!("unknown instance: {id}")})),
        )
            .into_response();
    }
    mgr.send_control(&id, minewrap_core::Control::Stop);
    mgr.send_control(&id, minewrap_core::Control::Start);
    Json(json!({"ok": true})).into_response()
}

async fn instance_command(
    axum::extract::Extension((mgr, auth)): axum::extract::Extension<(
        Arc<InstanceManager>,
        TokenAuth,
    )>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl axum::response::IntoResponse {
    if let Err(e) = check_auth(&auth, &headers) {
        return e.into_response();
    }
    if mgr.config_of(&id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": format!("unknown instance: {id}")})),
        )
            .into_response();
    }
    let command = body.get("command").and_then(|c| c.as_str()).unwrap_or("");
    let trimmed = command.replace(['\r', '\n'], "").trim().to_string();
    if trimmed.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": "command must not be empty"})),
        )
            .into_response();
    }
    let ok = mgr.send_control(&id, minewrap_core::Control::SendCommand(trimmed));
    Json(json!({"accepted": ok})).into_response()
}

async fn instance_logs(
    axum::extract::Extension((mgr, auth)): axum::extract::Extension<(
        Arc<InstanceManager>,
        TokenAuth,
    )>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl axum::response::IntoResponse {
    if let Err(e) = check_auth(&auth, &headers) {
        return e.into_response();
    }
    if mgr.config_of(&id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": format!("unknown instance: {id}")})),
        )
            .into_response();
    }
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(200usize);
    let since = params
        .get("since_seq")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u64);
    let lines = mgr.backfill(&id, limit, since);
    let first = lines.first().map(|l| l.seq);
    let last = lines.last().map(|l| l.seq);
    Json(json!({
        "instance_id": id,
        "first_seq": first,
        "last_seq": last,
        "lines": lines,
    }))
    .into_response()
}

/// Config view: secrets are write-only (`webhook_set: bool`).
async fn get_config(
    axum::extract::Extension((mgr, auth)): axum::extract::Extension<(
        Arc<InstanceManager>,
        TokenAuth,
    )>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    if let Err(e) = check_auth(&auth, &headers) {
        return e.into_response();
    }
    match mgr.config_of(&id) {
        Some(cfg) => {
            let view = json!({
                "id": cfg.id,
                "name": cfg.display_name(),
                "work_dir": cfg.work_dir,
                "java_path": cfg.java_path,
                "jvm_args": cfg.jvm_args,
                "launch_args": cfg.launch_args,
                "autostart": cfg.autostart,
                "probe": cfg.probe,
                "restart": cfg.restart,
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
            });
            Json(view).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": format!("unknown instance: {id}")})),
        )
            .into_response(),
    }
}

async fn update_config(
    axum::extract::Extension((mgr, auth)): axum::extract::Extension<(
        Arc<InstanceManager>,
        TokenAuth,
    )>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl axum::response::IntoResponse {
    if let Err(e) = check_auth(&auth, &headers) {
        return e.into_response();
    }
    // Patch + persist (0600, v1 config_store parity). The running
    // core picks the new config up on its next spawn; probe/notifier
    // rebuild happens there.
    match mgr.patch_instance(&id, body) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"detail": e}))).into_response(),
    }
}

async fn discord_test(
    axum::extract::Extension((mgr, auth)): axum::extract::Extension<(
        Arc<InstanceManager>,
        TokenAuth,
    )>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    if let Err(e) = check_auth(&auth, &headers) {
        return e.into_response();
    }
    let cfg = match mgr.config_of(&id) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"detail": format!("unknown instance: {id}")})),
            )
                .into_response();
        }
    };
    if !cfg.discord.enabled || !cfg.discord.webhook_set() {
        return Json(
            json!({"ok": false, "error": "discord is disabled or no webhook is configured"}),
        )
        .into_response();
    }
    let snapshot = mgr
        .snapshot_of(&id)
        .unwrap_or(minewrap_core::InstanceSnapshot {
            id: id.clone(),
            name: cfg.display_name().to_string(),
            combined_state: "offline".to_string(),
            process: minewrap_core::ProcessStatus {
                state: minewrap_core::ProcessState::Offline,
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
            probe: minewrap_core::ProbeInfo::default(),
            updated_at: chrono::Utc::now(),
        });
    let notifier = minewrap_core::Notifier::new(
        cfg.discord.clone(),
        crate::discord_service::shared_http().clone(),
        mgr.data_dir().join("discord"),
        id.clone(),
    );
    let result = notifier.send_test(&snapshot).await;
    Json(json!({"ok": result.ok, "error": result.error})).into_response()
}

async fn read_properties(
    axum::extract::Extension((mgr, auth)): axum::extract::Extension<(
        Arc<InstanceManager>,
        TokenAuth,
    )>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    if let Err(e) = check_auth(&auth, &headers) {
        return e.into_response();
    }
    let dir = match mgr.work_dir_of(&id) {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"detail": format!("unknown instance: {id}")})),
            )
                .into_response();
        }
    };
    let path = minewrap_core::properties::properties_path(&dir);
    match std::fs::read_to_string(&path) {
        Ok(text) => Json(json!({"text": text, "errors": []})).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": format!("server.properties: {e}")})),
        )
            .into_response(),
    }
}

async fn write_properties(
    axum::extract::Extension((mgr, auth)): axum::extract::Extension<(
        Arc<InstanceManager>,
        TokenAuth,
    )>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl axum::response::IntoResponse {
    if let Err(e) = check_auth(&auth, &headers) {
        return e.into_response();
    }
    let dir = match mgr.work_dir_of(&id) {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"detail": format!("unknown instance: {id}")})),
            )
                .into_response();
        }
    };
    let text = body.get("text").and_then(|t| t.as_str()).unwrap_or("");
    let doc = minewrap_core::PropertiesDocument::parse(text);
    let mut errors = Vec::new();
    for line in &doc.lines {
        if let minewrap_core::properties::Line::Property(entry) = line {
            if let Err(e) = minewrap_core::validate_property(&entry.key, &entry.value) {
                errors.push(format!("{}: {}", entry.key, e));
            }
        }
    }
    if !errors.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": errors.join("; ")})),
        )
            .into_response();
    }
    let path = minewrap_core::properties::properties_path(&dir);
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"detail": e.to_string()})),
            )
                .into_response();
        }
    }
    let tmp = path.with_extension("properties.tmp");
    if let Err(e) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"detail": e.to_string()})),
        )
            .into_response();
    }
    Json(json!({"ok": true})).into_response()
}

async fn list_mods(
    axum::extract::Extension((mgr, auth)): axum::extract::Extension<(
        Arc<InstanceManager>,
        TokenAuth,
    )>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    if let Err(e) = check_auth(&auth, &headers) {
        return e.into_response();
    }
    let dir = match mgr.work_dir_of(&id) {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"detail": format!("unknown instance: {id}")})),
            )
                .into_response();
        }
    };
    match minewrap_core::mods::list_mods(&dir) {
        Ok(entries) => Json(json!(entries)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"detail": e})),
        )
            .into_response(),
    }
}

async fn toggle_mod(
    axum::extract::Extension((mgr, auth)): axum::extract::Extension<(
        Arc<InstanceManager>,
        TokenAuth,
    )>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl axum::response::IntoResponse {
    if let Err(e) = check_auth(&auth, &headers) {
        return e.into_response();
    }
    let dir = match mgr.work_dir_of(&id) {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"detail": format!("unknown instance: {id}")})),
            )
                .into_response();
        }
    };
    let name = body.get("name").and_then(|n| n.as_str()).unwrap_or("");
    match minewrap_core::mods::toggle_mod(&dir, name) {
        Ok(enabled) => Json(json!({"ok": true, "enabled": enabled})).into_response(),
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"detail": e}))).into_response(),
    }
}

// --- WebSocket handlers (lives here to keep routing together; impl in ws.rs) ---
pub mod ws {
    use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
    use axum::http::HeaderMap;
    use axum::response::IntoResponse;
    use serde_json::json;

    use crate::auth::TokenAuth;
    use crate::instances::InstanceManager;

    pub async fn console_ws(
        ws: WebSocketUpgrade,
        headers: HeaderMap,
        axum::extract::Extension((mgr, auth)): axum::extract::Extension<(
            std::sync::Arc<InstanceManager>,
            TokenAuth,
        )>,
        axum::extract::Path(instance_id): axum::extract::Path<String>,
    ) -> impl IntoResponse {
        // Bearer at the WS handshake (v1 parity): deny before accept.
        let token = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(crate::auth::bearer_from_header)
            .unwrap_or_default();
        if !auth.verify(&token) {
            return (axum::http::StatusCode::UNAUTHORIZED, "invalid token").into_response();
        }
        ws.on_upgrade(move |socket| handle_console(socket, mgr, instance_id))
    }

    async fn handle_console(
        socket: WebSocket,
        mgr: std::sync::Arc<InstanceManager>,
        instance_id: String,
    ) {
        use futures_util::{SinkExt, StreamExt};
        let (mut sender, mut receiver) = socket.split();
        let mut sub = mgr.subscribe(&instance_id);
        // Backfill + snapshot on connect (v1 parity).
        let lines = mgr.backfill(&instance_id, 200, 0);
        let hello = json!({
            "type": "log",
            "instance_id": instance_id,
            "lines": lines,
        });
        let _ = sender.send(Message::Text(hello.to_string().into())).await;
        if let Some(snap) = mgr.snapshot_of(&instance_id) {
            let msg = json!({"type": "snapshot", "snapshot": snap});
            let _ = sender.send(Message::Text(msg.to_string().into())).await;
        }
        loop {
            tokio::select! {
                msg = receiver.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            // Client -> daemon: {"type":"command","command":"say hi"}
                            let v: serde_json::Value = serde_json::from_str(&text).unwrap_or(json!({}));
                            if v.get("type").and_then(|t| t.as_str()) == Some("command") {
                                let cmd = v.get("command").and_then(|c| c.as_str()).unwrap_or("");
                                mgr.send_control(&instance_id, minewrap_core::Control::SendCommand(cmd.to_string()));
                                let res = json!({"type": "command_result", "accepted": true});
                                let _ = sender.send(Message::Text(res.to_string().into())).await;
                            }
                        }
                        Some(Ok(Message::Close(_))) | None => break,
                        _ => {}
                    }
                }
                ev = sub.rx.recv() => {
                    match ev {
                        Some(minewrap_core::Event::LogLine(le)) => {
                            let msg = json!({"type": "log", "instance_id": le.instance_id, "line": le.line});
                            if sender.send(Message::Text(msg.to_string().into())).await.is_err() { break; }
                        }
                        Some(minewrap_core::Event::StateChange(sc)) => {
                            let msg = json!({"type": "state_change", "instance_id": sc.instance_id, "old_state": sc.old_state, "new_state": sc.new_state, "reason": sc.reason});
                            if sender.send(Message::Text(msg.to_string().into())).await.is_err() { break; }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    pub async fn events_ws(
        ws: WebSocketUpgrade,
        headers: HeaderMap,
        axum::extract::Extension((mgr, auth)): axum::extract::Extension<(
            std::sync::Arc<InstanceManager>,
            TokenAuth,
        )>,
    ) -> impl IntoResponse {
        let token = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(crate::auth::bearer_from_header)
            .unwrap_or_default();
        if !auth.verify(&token) {
            return (axum::http::StatusCode::UNAUTHORIZED, "invalid token").into_response();
        }
        ws.on_upgrade(move |socket| handle_events(socket, mgr))
    }

    async fn handle_events(socket: WebSocket, mgr: std::sync::Arc<InstanceManager>) {
        use futures_util::{SinkExt, StreamExt};
        let (mut sender, mut receiver) = socket.split();
        let mut sub = mgr.subscribe(minewrap_core::events::WILDCARD);
        let mut changes = mgr.subscribe_changes();
        let hello = json!({"type": "hello_ok"});
        let _ = sender.send(Message::Text(hello.to_string().into())).await;
        // Server-side log batching (task B4): parity with the v1.9.3
        // GUI forwarder — 100 lines / 40 ms per instance message.
        let mut pending: Vec<(String, minewrap_core::LogLine)> = Vec::new();
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(40));
        loop {
            tokio::select! {
                msg = receiver.next() => {
                    match msg {
                        Some(Ok(Message::Close(_))) | None => break,
                        _ => {}
                    }
                }
                ev = sub.rx.recv() => {
                    match ev {
                        Some(minewrap_core::Event::StateChange(sc)) => {
                            let msg = json!({"type": "state_change", "instance_id": sc.instance_id, "old_state": sc.old_state, "new_state": sc.new_state, "reason": sc.reason});
                            if sender.send(Message::Text(msg.to_string().into())).await.is_err() { break; }
                        }
                        Some(minewrap_core::Event::LogLine(le)) => {
                            pending.push((le.instance_id.clone(), le.line));
                        }
                        Some(minewrap_core::Event::RestartLoop(rl)) => {
                            let msg = json!({"type": "restart_loop", "instance_id": rl.instance_id, "crash_count": rl.crash_count});
                            if sender.send(Message::Text(msg.to_string().into())).await.is_err() { break; }
                        }
                        None => break,
                    }
                }
                _ = changes.recv() => {
                    // Instance added/removed (task B4): tell clients to
                    // refresh their instance lists.
                    let msg = json!({"type": "instances_changed"});
                    if sender.send(Message::Text(msg.to_string().into())).await.is_err() { break; }
                }
                _ = interval.tick() => {
                    if pending.is_empty() {
                        continue;
                    }
                    use std::collections::HashMap;
                    let mut by_instance: HashMap<String, Vec<minewrap_core::LogLine>> = HashMap::new();
                    for (iid, line) in pending.drain(..) {
                        by_instance.entry(iid).or_default().push(line);
                    }
                    for (instance_id, mut lines) in by_instance {
                        for chunk in lines.chunks_mut(100) {
                            let first = chunk.first().map(|l| l.seq).unwrap_or(0);
                            let last = chunk.last().map(|l| l.seq).unwrap_or(0);
                            let msg = json!({
                                "type": "log_batch",
                                "instance_id": instance_id,
                                "lines": chunk,
                                "first_seq": first,
                                "last_seq": last,
                            });
                            if sender.send(Message::Text(msg.to_string().into())).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        }
    }
}
