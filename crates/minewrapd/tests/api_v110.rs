//! Integration tests for the v1.10.0 REST additions and the `/events`
//! WebSocket (task B4): instance CRUD, java detection, work-dir scan,
//! and log batching through a 2000-line burst.
//!
//! The "server" is a fake java: a shell script that prints a burst of
//! lines, says `Done`, then streams slowly — same shape as the memtest
//! `fakejava` stub (outside this repo per the clean-repo policy; this
//! test builds its own inline copy under a temp dir).

use std::sync::Arc;
use std::time::Duration;

use minewrap_core::config::{DaemonConfig, InstanceConfig};
use minewrapd::{InstanceManager, auth::TokenAuth, create_router};

fn fakejava_script(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("fakejava");
    std::fs::write(
        &path,
        "#!/bin/sh\nfor i in $(seq 1 2000); do echo \"[Server thread/INFO]: line $i\"; done\necho 'Done (1.23s)! For help, type \"help\"'\nwhile true; do sleep 1; echo tick; done\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

fn instance(id: &str, work_dir: &str, java: &str) -> InstanceConfig {
    InstanceConfig {
        id: id.into(),
        name: String::new(),
        work_dir: work_dir.into(),
        java_path: java.into(),
        jvm_args: vec![],
        launch_args: vec![],
        autostart: false,
        probe: Default::default(),
        restart: Default::default(),
        rcon: None,
        discord: Default::default(),
    }
}

fn unique_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEED: AtomicU64 = AtomicU64::new(0);
    let n = SEED.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("minewrapd-it-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let _ = label;
    dir
}

struct TestDaemon {
    port: u16,
    token: String,
    #[allow(dead_code)]
    dir: std::path::PathBuf,
}

async fn start_daemon(instances: Vec<InstanceConfig>) -> TestDaemon {
    let dir = unique_dir("it");
    let java = fakejava_script(&dir).to_string_lossy().to_string();

    let mut cfg = DaemonConfig::default();
    cfg.daemon.host = "127.0.0.1".into();
    cfg.daemon.data_dir = dir.join("data").to_string_lossy().to_string();
    cfg.daemon.status_poll_interval = 3600.0; // keep the poller quiet
    for inst in instances {
        let mut inst = inst;
        if inst.java_path == "AUTO" {
            inst.java_path = java.clone();
        }
        if inst.work_dir == "AUTO" {
            inst.work_dir = dir.join("work").to_string_lossy().to_string();
            std::fs::create_dir_all(&inst.work_dir).unwrap();
        }
        cfg.instances.push(inst);
    }

    let mgr = Arc::new(InstanceManager::load(
        cfg,
        Arc::new(minewrap_core::EventBus::new()),
        Some(dir.join("minewrap.toml").to_string_lossy().to_string()),
    ));
    let token = minewrapd::auth::TokenAuth::generate();
    let auth = TokenAuth::new(token.clone());
    let router = create_router(mgr, auth);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    TestDaemon { port, token, dir }
}

impl TestDaemon {
    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
    fn auth(&self) -> String {
        format!("Bearer {}", self.token)
    }
}

#[tokio::test]
async fn crud_java_scan_and_events_log_batch() {
    let d = start_daemon(vec![instance("srv", "AUTO", "AUTO")]).await;
    let client = reqwest::Client::new();

    // health is public
    let health: serde_json::Value = client
        .get(format!("{}/health", d.base()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["api_version"], "1.0");

    // unauthorized without token
    let r = client
        .get(format!("{}/api/v1/instances", d.base()))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);

    // list contains the seeded instance
    let list: serde_json::Value = client
        .get(format!("{}/api/v1/instances", d.base()))
        .header("Authorization", &d.auth())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list[0]["id"], "srv");

    // POST /instances: create + duplicate -> 201/409
    let created = client
        .post(format!("{}/api/v1/instances", d.base()))
        .header("Authorization", &d.auth())
        .json(&serde_json::json!({
            "id": "srv2",
            "work_dir": "/tmp",
            "java_path": "java",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201);
    let dup = client
        .post(format!("{}/api/v1/instances", d.base()))
        .header("Authorization", &d.auth())
        .json(&serde_json::json!({
            "id": "srv2",
            "work_dir": "/tmp",
            "java_path": "java",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(dup.status(), 409);

    // GET /java returns an array
    let java: serde_json::Value = client
        .get(format!("{}/api/v1/java", d.base()))
        .header("Authorization", &d.auth())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(java.as_array().is_some());

    // POST /scan-workdir on a real temp dir
    let scan: serde_json::Value = client
        .post(format!("{}/api/v1/scan-workdir", d.base()))
        .header("Authorization", &d.auth())
        .json(&serde_json::json!({"work_dir": "/tmp"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(scan.get("candidates").is_some());
    assert!(scan.get("report").is_some());

    // Open the /events WS, then start the server: we must receive
    // state_change(online) and log batches with continuous seq through
    // the 2000-line burst (B4 AC).
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::handshake::client::generate_key;
    use tokio_tungstenite::tungstenite::http::Request;

    let ws_url = format!("ws://127.0.0.1:{}/events", d.port);
    let req = Request::builder()
        .uri(&ws_url)
        .header("Host", format!("127.0.0.1:{}", d.port))
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", generate_key())
        .header("Authorization", d.auth())
        .body(())
        .unwrap();
    let (mut ws, _resp) = tokio_tungstenite::connect_async(req).await.unwrap();
    let _ = ws
        .send(tokio_tungstenite::tungstenite::Message::text("hello"))
        .await;

    let start = client
        .post(format!("{}/api/v1/instances/srv/start", d.base()))
        .header("Authorization", &d.auth())
        .send()
        .await
        .unwrap();
    assert!(start.status().is_success());

    let mut last_seq: u64 = 0;
    let mut saw_online = false;
    let mut total_lines = 0usize;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while tokio::time::Instant::now() < deadline {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next()).await;
        let msg = match msg {
            Ok(Some(Ok(m))) => m,
            _ => break,
        };
        let text = match msg.into_text() {
            Ok(t) => t.to_string(),
            Err(_) => continue,
        };
        let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        match v["type"].as_str() {
            Some("state_change") => {
                if v["new_state"] == "online" {
                    saw_online = true;
                }
            }
            Some("log_batch") => {
                let lines = v["lines"].as_array().unwrap();
                assert!(lines.len() <= 100, "batch cap is 100 lines");
                for line in lines {
                    let seq = line["seq"].as_u64().unwrap();
                    if last_seq > 0 {
                        assert_eq!(seq, last_seq + 1, "seq must be continuous, no gaps");
                    }
                    last_seq = seq;
                }
                total_lines += lines.len();
                if saw_online && total_lines >= 2001 {
                    break; // burst + Done line received in order
                }
            }
            _ => {}
        }
    }
    assert!(saw_online, "must observe the online state change");
    assert!(
        total_lines >= 2001,
        "must receive the full 2000-line burst + Done (got {total_lines})"
    );

    // Stop it, then DELETE: 200 and the instance disappears from list.
    let _ = client
        .post(format!("{}/api/v1/instances/srv/stop", d.base()))
        .header("Authorization", &d.auth())
        .send()
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let deleted = client
        .delete(format!("{}/api/v1/instances/srv2", d.base()))
        .header("Authorization", &d.auth())
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), 200);
    let missing = client
        .delete(format!("{}/api/v1/instances/ghost", d.base()))
        .header("Authorization", &d.auth())
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);

    let list2: serde_json::Value = client
        .get(format!("{}/api/v1/instances", d.base()))
        .header("Authorization", &d.auth())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ids: Vec<&str> = list2
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["id"].as_str().unwrap())
        .collect();
    assert!(!ids.contains(&"srv2"), "deleted instance must be gone");

    let _ = ws.close(None).await;
    let _ = std::fs::remove_dir_all(&d.dir);
}

#[tokio::test]
async fn config_rename_shows_immediately_without_restart() {
    // BUG-D1: PUT /config changing `name` must be visible in
    // GET /instances and GET /instances/{id} at once — `summary_of`/
    // `snapshot_of` read the stale `InstanceState.name` cache otherwise.
    let d = start_daemon(vec![instance("srv", "AUTO", "AUTO")]).await;
    let client = reqwest::Client::new();
    let auth = d.auth();

    let list: serde_json::Value = client
        .get(format!("{}/api/v1/instances", d.base()))
        .header("Authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list[0]["id"], "srv");
    assert_eq!(list[0]["name"], "srv"); // name falls back to id

    let r = client
        .put(format!("{}/api/v1/instances/srv/config", d.base()))
        .header("Authorization", &auth)
        .json(&serde_json::json!({"name": "Survival Renamed"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "PUT /config must accept the rename");
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body.get("ok").and_then(|v| v.as_bool()), Some(true));

    // No restart: the list endpoint must show the new name immediately.
    let list: serde_json::Value = client
        .get(format!("{}/api/v1/instances", d.base()))
        .header("Authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list[0]["name"], "Survival Renamed");

    // The per-instance snapshot agrees.
    let snap: serde_json::Value = client
        .get(format!("{}/api/v1/instances/srv", d.base()))
        .header("Authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(snap["id"], "srv");
    assert_eq!(snap["name"], "Survival Renamed");

    let _ = std::fs::remove_dir_all(&d.dir);
}
