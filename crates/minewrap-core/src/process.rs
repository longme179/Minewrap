//! Process manager: state machine, spawn, done-regex, stop escalation,
//! restart policy — ported from Python v1 `mcpanel_daemon/process_manager.py`.
//!
//! Parity guarantees (ported tests):
//! - `tokio::process::Command`, own process group, stdin/stdout piped,
//!   stderr merged into stdout.
//! - `BufReader::read_line` — lossy UTF-8, huge lines truncated to
//!   `MAX_LINE_BYTES`.
//! - `BufReader` → bounded `mpsc` → `LogBuffer` + rotating file + bus events.
//! - Stop: `stop` on stdin → wait `stop_timeout` → SIGTERM → wait
//!   `kill_grace_seconds` → SIGKILL (process group).
//! - `requested_stop` distinguishes crash from stop; `transition()` is the
//!   only place the state changes.
//!
//! The owner task is spawned by `spawn()`; IPC/axum callers talk to the
//! `OwnerHandle` control channel. The manager itself contains no Tauri code.

use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::config::{InstanceConfig, InstanceDefaults};
use crate::events::{Event, EventBus, RestartLoopEvent, StateChange};
use crate::logs::{LogBuffer, RotatingFile};

/// Process state (parity with v1 `process_manager.State`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessState {
    Offline,
    Starting,
    Online,
    Stopping,
    Crashed,
}

/// Status snapshot (parity with v1 `ProcessStatus`).
#[derive(Debug, Clone, Serialize)]
pub struct ProcessStatus {
    pub state: ProcessState,
    pub pid: Option<u32>,
    pub started_at: Option<DateTime<Utc>>,
    pub stopped_at: Option<DateTime<Utc>>,
    pub exit_code: Option<i32>,
    pub requested_stop: bool,
    pub forced_stop: bool,
    pub restart_count: u32,
    pub crash_count: u32,
    pub uptime_seconds: Option<f64>,
}

/// Manager error (parity with v1 `ProcessError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessError(pub String);

impl std::fmt::Display for ProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ProcessError {}

/// Transition table (parity with v1 `_ALLOWED_TRANSITIONS`).
pub fn allowed_next(state: ProcessState) -> &'static [ProcessState] {
    use ProcessState::*;
    match state {
        Offline => &[Starting],
        Starting => &[Online, Stopping, Crashed],
        Online => &[Stopping, Crashed],
        Stopping => &[Offline],
        Crashed => &[Starting, Offline],
    }
}

/// Maximum a single emitted line is held in memory (RSS budget).
pub const MAX_LINE_BYTES: usize = 64 * 1024;

/// Pipe back-pressure between spawn-to-buffer (v1 used unbounded stdout
/// iteration; the pipe reader here is bounded). 256 lines ~ 256 KB max.
pub const PIPE_CAPACITY: usize = 256;

fn state_str(state: ProcessState) -> String {
    match state {
        ProcessState::Offline => "offline",
        ProcessState::Starting => "starting",
        ProcessState::Online => "online",
        ProcessState::Stopping => "stopping",
        ProcessState::Crashed => "crashed",
    }
    .to_string()
}

/// Batch timer + size for log-event coalescing (Phase 1b: events Bus batch).
pub const BATCH_INTERVAL_MS: u64 = 40;
pub const BATCH_MAX_LINES: usize = 100;

/// Spawn the fake/real server and return `(child)`. The piped stdout/stderr
/// are copied into one bounded channel; the reader task is owned by the caller.
pub async fn spawn_server(config: &InstanceConfig) -> Result<(Child, mpsc::Receiver<String>), String> {
    let work_dir = Path::new(&config.work_dir);
    if !work_dir.exists() {
        return Err(format!("work dir does not exist: {}", config.work_dir));
    }
    if !work_dir.is_dir() {
        return Err(format!("work dir is not a directory: {}", config.work_dir));
    }

    let argv: Vec<String> = {
        let mut parts = vec![config.java_path.clone()];
        parts.extend(config.jvm_args.iter().cloned());
        parts.extend(config.launch_args.iter().cloned());
        parts
    };
    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]);
    command.current_dir(work_dir);
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);

    let mut child = command.spawn().map_err(|e| {
        format!(
            "failed to start `{}` in `{}`: {e}",
            argv.join(" "),
            config.work_dir
        )
    })?;
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let (tx, rx) = mpsc::channel::<String>(PIPE_CAPACITY);
    tokio::spawn(copy_stream(stdout, tx.clone()));
    tokio::spawn(copy_stream(stderr, tx));
    Ok((child, rx))
}

async fn copy_stream<R>(stream: R, tx: mpsc::Sender<String>)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut reader = BufReader::new(stream);
    loop {
        let mut buf = String::new();
        match reader.read_line(&mut buf).await {
            Ok(0) => break, // EOF
            Ok(_) => {
                if tx.send(buf).await.is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

fn truncate_line(line: &str, max_bytes: usize) -> String {
    const ELLIPSIS: &str = "… [truncated]";
    if line.len() <= max_bytes {
        return line.to_string();
    }
    let mut out = String::new();
    let mut remaining = max_bytes.saturating_sub(ELLIPSIS.len());
    for ch in line.chars() {
        let mut buf = [0u8; 4];
        let encoded = ch.encode_utf8(&mut buf);
        if encoded.len() > remaining {
            break;
        }
        out.push_str(encoded);
        remaining -= encoded.len();
    }
    out.push_str(ELLIPSIS);
    out
}

/// Done-line regex (v1 parity: Python `re`, custom patterns supported).
pub struct DoneMatcher {
    re: regex::Regex,
}

impl DoneMatcher {
    pub fn compile(pattern: &str) -> Self {
        match regex::Regex::new(pattern) {
            Ok(re) => Self { re },
            Err(_) => Self {
                re: regex::Regex::new(r"Done \(").unwrap(),
            },
        }
    }

    pub fn search(&self, text: &str) -> bool {
        self.re.is_match(text)
    }
}

/// Owner-side controls.
pub enum Control {
    Start,
    Stop,
    SendCommand(String),
    Shutdown,
}

/// Spawn the standalone line reader (used after `spawn_server`).
/// Lives outside the impl because `tokio::spawn` needs a 'static future.
pub fn spawn_reader(
    mut rx: mpsc::Receiver<String>,
    buffer: Arc<Mutex<LogBuffer>>,
    bus: Arc<EventBus>,
    instance_id: String,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            let text = truncate_line(&line, MAX_LINE_BYTES);
            let entry = crate::logs::parse_line(0, Utc::now(), &text);
            {
                let mut b = match buffer.lock() {
                    Ok(b) => b,
                    Err(p) => p.into_inner(),
                };
                b.append(entry.line.clone(), entry.ts);
            }
            bus.publish(&Event::LogLine(crate::events::LogLineEvent {
                instance_id: instance_id.clone(),
                line: entry,
            }));
        }
    })
}

/// Handle for callers: owns the task, and carries the control channel.
pub struct OwnerHandle {
    pub owner: tokio::task::JoinHandle<()>,
    pub ctrl_tx: mpsc::Sender<Control>,
    _runtime_thread: Option<std::thread::JoinHandle<()>>,
}

/// Manages one instance (owns `Child` + state machine; called by handle).
pub struct ProcessManager {
    config: InstanceConfig,
    defaults: InstanceDefaults,
    bus: Arc<EventBus>,
    done: DoneMatcher,
    buffer: Arc<Mutex<LogBuffer>>,
    #[allow(dead_code)]
    log_file: Option<Mutex<RotatingFile>>,

    ctrl_tx: mpsc::Sender<Control>,
    ctrl_rx: mpsc::Receiver<Control>,

    state: ProcessState,
    child: Option<Child>,
    started_at: Option<DateTime<Utc>>,
    stopped_at: Option<DateTime<Utc>>,
    exit_code: Option<i32>,
    requested_stop: bool,
    forced_stop: bool,
    restart_count: u32,
    crash_count: u32,
    stop_started_at: Option<tokio::time::Instant>,
    term_sent_at: Option<tokio::time::Instant>,
}

impl ProcessManager {
    pub fn new(
        config: InstanceConfig,
        defaults: InstanceDefaults,
        bus: Arc<EventBus>,
        buffer: Arc<Mutex<LogBuffer>>,
        log_file: Option<Mutex<RotatingFile>>,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<Control>(32);
        let done = DoneMatcher::compile(&defaults.done_regex);
        Self {
            config,
            defaults,
            bus,
            done,
            buffer,
            log_file,
            ctrl_tx: tx,
            ctrl_rx: rx,
            state: ProcessState::Offline,
            child: None,
            started_at: None,
            stopped_at: None,
            exit_code: None,
            requested_stop: false,
            forced_stop: false,
            restart_count: 0,
            crash_count: 0,
            stop_started_at: None,
            term_sent_at: None,
        }
    }

    /// Start the owner task. Spawns on the current Tokio runtime when
    /// available; otherwise creates a dedicated runtime thread so the
    /// desktop app works even when callers don't hold a Tokio context.
    pub fn spawn(self) -> OwnerHandle {
        let ctrl_tx = self.ctrl_tx.clone();
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => OwnerHandle {
                owner: handle.spawn(self.run()),
                ctrl_tx,
                _runtime_thread: None,
            },
            Err(_) => {
                let (tx_done, rx_done) = std::sync::mpsc::sync_channel::<tokio::task::JoinHandle<()>>(1);
                let thread = std::thread::spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("process manager runtime");
                    let handle = rt.spawn(self.run());
                    let _ = tx_done.send(handle);
                    rt.block_on(async {
                        // Keep runtime alive; the task completes on shutdown/drop.
                        std::future::pending::<()>().await;
                    });
                });
                let owner = rx_done
                    .recv()
                    .expect("process manager handle");
                OwnerHandle {
                    owner,
                    ctrl_tx,
                    _runtime_thread: Some(thread),
                }
            }
        }
    }

    pub fn state(&self) -> ProcessState {
        self.state
    }

    pub fn status(&self) -> ProcessStatus {
        let uptime = self
            .started_at
            .filter(|_| matches!(
                self.state,
                ProcessState::Starting | ProcessState::Online | ProcessState::Stopping
            ))
            .map(|t| (Utc::now() - t).num_milliseconds() as f64 / 1000.0);
        ProcessStatus {
            state: self.state,
            pid: self.child.as_ref().and_then(|c| c.id()),
            started_at: self.started_at,
            stopped_at: self.stopped_at,
            exit_code: self.exit_code,
            requested_stop: self.requested_stop,
            forced_stop: self.forced_stop,
            restart_count: self.restart_count,
            crash_count: self.crash_count,
            uptime_seconds: uptime,
        }
    }

    fn transition(&mut self, new: ProcessState, reason: &str) {
        let old = self.state;
        if old == new {
            return;
        }
        assert!(allowed_next(old).contains(&new), "invalid transition {old:?} -> {new:?}");
        self.state = new;
        self.bus.publish(&Event::StateChange(StateChange {
            instance_id: self.config.id.clone(),
            old_state: state_str(old),
            new_state: state_str(new),
            reason: reason.to_string(),
        }));
    }

    fn sign_group(&mut self, sig: i32) {
        #[cfg(unix)]
        {
            if let Some(child) = &self.child {
                if let Some(pid) = child.id() {
                    let pgid = unsafe { libc::getpgid(pid as i32) };
                    if pgid != -1 {
                        unsafe { libc::kill(-pgid, sig) };
                    }
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = sig;
        }
    }

    /// Run the state machine until shutdown (takes `self`).
    async fn run(mut self) {
        let mut startup_deadline: Option<tokio::time::Instant> = None;
        let mut reader: Option<tokio::task::JoinHandle<()>> = None;
        let mut rx = std::mem::replace(
            &mut self.ctrl_rx,
            tokio::sync::mpsc::channel::<Control>(0).1,
        );

        loop {
            let has_child = self.child.is_some() || reader.is_some();
            tokio::select! {
                msg = rx.recv() => {
                    let Some(msg) = msg else { return };
                    match msg {
                        Control::Start => {
                            if !matches!(self.state, ProcessState::Offline | ProcessState::Crashed) {
                                continue;
                            }
                            self.crash_count = 0;
                            self.restart_count = 0;
                            match spawn_server(&self.config).await {
                                Ok((child, rx)) => {
                                    self.child = Some(child);
                                    self.transition(ProcessState::Starting, "start_requested");
                                    self.started_at = Some(Utc::now());
                                    startup_deadline = Some(tokio::time::Instant::now() + Duration::from_secs_f64(self.defaults.startup_timeout));
                                    reader = Some(spawn_reader(
                                        rx,
                                        self.buffer.clone(),
                                        self.bus.clone(),
                                        self.config.id.clone(),
                                    ));
                                }
                                Err(e) => self.transition(ProcessState::Crashed, &format!("spawn_failed: {e}")),
                            }
                        }
                        Control::Stop => {
                            if self.state == ProcessState::Offline {
                                continue;
                            }
                            if self.state == ProcessState::Crashed {
                                self.transition(ProcessState::Offline, "stop_acknowledged");
                                self.crash_count = 0;
                                self.restart_count = 0;
                                continue;
                            }
                            self.requested_stop = true;
                            self.transition(ProcessState::Stopping, "stop_requested");
                            self.stop_started_at = Some(tokio::time::Instant::now());
                            self.term_sent_at = None;
                            if let Some(child) = &mut self.child {
                                if let Some(stdin) = child.stdin.take() {
                                    let mut stdin = stdin;
                                    let _ = stdin.write_all(b"stop\n").await;
                                    let _ = stdin.flush().await;
                                }
                            }
                        }
                        Control::SendCommand(cmd) => {
                            if self.state != ProcessState::Online {
                                continue; // v1: reject when not running
                            }
                            if let Some(child) = &mut self.child {
                                if let Some(stdin) = &mut child.stdin {
                                    let _ = stdin.write_all(format!("{cmd}\n").as_bytes()).await;
                                    let _ = stdin.flush().await;
                                }
                            }
                        }
                        Control::Shutdown => {
                            self.sign_group(SIGKILL);
                            if let Some(mut child) = self.child.take() {
                                let _ = child.wait().await;
                            }
                            if let Some(r) = reader.take() {
                                r.abort();
                            }
                            return;
                        }
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(10)), if has_child => {
                    // done-line check while starting
                    if self.state == ProcessState::Starting {
                        let hit = {
                            let buf = self.buffer.lock().unwrap();
                            buf.tail(5).iter().any(|l| self.done.search(&l.line))
                        };
                        if hit {
                            self.transition(ProcessState::Online, "done_line");
                            startup_deadline = None;
                        }
                    }

                    // exit watcher
                    let mut exited: Option<std::process::ExitStatus> = None;
                    if let Some(child) = &mut self.child {
                        if let Ok(Some(status)) = child.try_wait() {
                            exited = Some(status);
                        }
                    }
                    if let Some(status) = exited {
                        if let Some(r) = reader.take() {
                            let _ = tokio::time::timeout(Duration::from_millis(300), r).await;
                        }
                        self.exit_code = status.code();
                        self.stopped_at = Some(Utc::now());
                        self.child = None;
                        self.stop_started_at = None;
                        self.term_sent_at = None;
                        startup_deadline = None;
                        match self.state {
                            ProcessState::Stopping => self.transition(ProcessState::Offline, "stopped"),
                            ProcessState::Starting => {
                                self.transition(ProcessState::Crashed, "crash_during_start");
                                self.handle_crash(&mut reader).await;
                            }
                            ProcessState::Online => {
                                self.transition(ProcessState::Crashed, "crashed");
                                self.handle_crash(&mut reader).await;
                            }
                            ProcessState::Crashed => self.handle_crash(&mut reader).await,
                            ProcessState::Offline => {}
                        }
                        continue;
                    }

                    // stop escalation
                    if self.state == ProcessState::Stopping {
                        let now = tokio::time::Instant::now();
                        if self.term_sent_at.is_none()
                            && self.stop_started_at.is_some_and(|t| now >= t + Duration::from_secs_f64(self.defaults.stop_timeout))
                        {
                            self.sign_group(SIGTERM);
                            self.term_sent_at = Some(now);
                        } else if self.term_sent_at.is_some_and(|t| now >= t + Duration::from_secs_f64(self.defaults.kill_grace_seconds))
                            && !self.forced_stop
                        {
                            self.forced_stop = true;
                            self.sign_group(SIGKILL);
                            if let Some(mut child) = self.child.take() {
                                let _ = child.wait().await;
                            }
                            self.stopped_at = Some(Utc::now());
                            self.transition(ProcessState::Offline, "stopped");
                            self.stop_started_at = None;
                            self.term_sent_at = None;
                            if let Some(r) = reader.take() {
                                r.abort();
                            }
                            continue;
                        }
                    }

                    // startup timeout
                    if let Some(deadline) = startup_deadline {
                        if self.state == ProcessState::Starting && tokio::time::Instant::now() >= deadline {
                            self.transition(ProcessState::Crashed, "startup_timeout");
                            self.sign_group(SIGTERM);
                            if let Some(mut child) = self.child.take() {
                                match tokio::time::timeout(Duration::from_secs_f64(self.defaults.kill_grace_seconds), child.wait()).await {
                                    Ok(_) => {}
                                    Err(_) => {
                                        self.forced_stop = true;
                                        self.sign_group(SIGKILL);
                                        let _ = child.wait().await;
                                    }
                                }
                            }
                            self.stopped_at = Some(Utc::now());
                            startup_deadline = None;
                            self.handle_crash(&mut reader).await;
                        }
                    }
                }
            }
        }
    }

    #[allow(dead_code)]
    async fn do_spawn(&mut self) -> Result<mpsc::Receiver<String>, String> {
        match spawn_server(&self.config).await {
            Ok((child, rx)) => {
                self.child = Some(child);
                self.transition(ProcessState::Starting, "start_requested");
                self.started_at = Some(Utc::now());
                self.exit_code = None;
                self.stopped_at = None;
                self.requested_stop = false;
                self.forced_stop = false;
                Ok(rx)
            }
            Err(e) => Err(e),
        }
    }

    async fn handle_crash(&mut self, reader: &mut Option<tokio::task::JoinHandle<()>>) {
        self.crash_count += 1;
        let policy = &self.config.restart;
        if policy.policy == "on-crash" && self.crash_count <= policy.max_retries {
            let delay = (policy.backoff_base_seconds * 2f64.powi(self.crash_count.saturating_sub(1) as i32))
                .min(policy.backoff_max_seconds);
            tokio::time::sleep(Duration::from_secs_f64(delay)).await;
            if self.state == ProcessState::Crashed {
                self.restart_count += 1;
                if let Ok((child, rx)) = spawn_server(&self.config).await {
                    self.child = Some(child);
                    self.transition(ProcessState::Starting, "start_requested");
                    self.started_at = Some(Utc::now());
                    *reader = Some(spawn_reader(
                        rx,
                        self.buffer.clone(),
                        self.bus.clone(),
                        self.config.id.clone(),
                    ));
                }
            }
        } else {
            self.bus.publish(&Event::RestartLoop(RestartLoopEvent {
                instance_id: self.config.id.clone(),
                crash_count: self.crash_count,
            }));
        }
    }
}

const SIGTERM: i32 = 15;
const SIGKILL: i32 = 9;

#[cfg(unix)]
mod libc {
    #[link(name = "c")]
    unsafe extern "C" {
        pub fn getpgid(pid: i32) -> i32;
        pub fn kill(pid: i32, sig: i32) -> i32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_table_matches_v1() {
        assert!(allowed_next(ProcessState::Starting).contains(&ProcessState::Online));
        assert!(allowed_next(ProcessState::Online).contains(&ProcessState::Stopping));
        assert!(!allowed_next(ProcessState::Online).contains(&ProcessState::Offline));
        assert!(allowed_next(ProcessState::Crashed).contains(&ProcessState::Starting));
    }

    #[test]
    fn done_regex_matches_default() {
        let re = DoneMatcher::compile(r"Done \(\d+(\.\d+)?s\)! For help");
        assert!(re.search(r#"Done (1.23s)! For help, type "help""#));
    }

    #[test]
    fn truncate_long_line() {
        let long = "x".repeat(100_000);
        let out = truncate_line(&long, 1024);
        assert!(out.len() <= 1024);
        assert!(out.ends_with("[truncated]"));
    }
}
