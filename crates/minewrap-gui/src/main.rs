//! minewrap-gui — pure-Rust Slint client for `minewrapd`.
//!
//! P1 chunk 1: daemon discovery/auto-spawn, 2 s instance-list polling,
//! console WebSocket with `since_seq` backfill, 1000-line ring with
//! per-level colours and Ctrl+C copy, Start/Stop + stdin command input.
//! Closing the window only closes the client; the daemon keeps servers up.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use minewrap_remote::RemoteClient;
use minewrap_remote::types::{InstanceSummaryDto, LogLineDto};
use slint::{Timer, TimerMode, VecModel};
use tokio::sync::{mpsc, watch};

use minewrap_gui::console::{self, PushOutcome, Ring};
use minewrap_gui::daemon;
use minewrap_gui::instances;
use minewrap_gui::runtime::{self, Cmd, UiMsg};

slint::include_modules!();

/// A line accepted by the ring but not yet pushed to the Slint model.
type PendingLine = (String, LogLineDto);

/// UI-thread state shared by callbacks and the 50 ms drain timer.
struct State {
    ring: Ring,
    pending: VecDeque<PendingLine>,
    instances: Vec<InstanceSummaryDto>,
    selected: Option<String>,
    line_model: Rc<VecModel<LogLine>>,
    inst_model: Rc<VecModel<InstRow>>,
    cmd_tx: mpsc::UnboundedSender<Cmd>,
    sel_tx: watch::Sender<Option<String>>,
    clipboard: Option<arboard::Clipboard>,
}

impl State {
    /// Switch the console to `id`, resetting the ring/backfill cursor.
    fn select(&mut self, ui: &App, id: &str) {
        if self.selected.as_deref() == Some(id) {
            return;
        }
        self.selected = Some(id.to_string());
        self.ring.reset();
        self.pending.clear();
        self.line_model.set_vec(Vec::new());
        self.sel_tx.send(Some(id.to_string())).ok();
        ui.set_selected_id(id.into());
        ui.set_sel_lo(-1);
        ui.set_sel_hi(-1);
        ui.set_stick(true);
        self.refresh_header(ui);
    }

    /// Rebuild the sidebar model, keeping (or choosing) a valid selection.
    fn apply_instances(&mut self, ui: &App, list: Vec<InstanceSummaryDto>) {
        self.instances = list;
        let rows: Vec<InstRow> = self
            .instances
            .iter()
            .map(|i| {
                let code = instances::state_code(&i.combined_state);
                InstRow {
                    id: i.id.as_str().into(),
                    name: if i.name.is_empty() {
                        i.id.as_str().into()
                    } else {
                        i.name.as_str().into()
                    },
                    state: instances::state_label(code).into(),
                    code,
                }
            })
            .collect();
        self.inst_model.set_vec(rows);

        let still_there = self
            .selected
            .as_ref()
            .is_some_and(|id| self.instances.iter().any(|i| &i.id == id));
        if !still_there {
            self.pending.clear();
            self.ring.reset();
            self.line_model.set_vec(Vec::new());
            self.selected = None;
            self.sel_tx.send(None).ok();
            ui.set_selected_id("".into());
            ui.set_sel_lo(-1);
            ui.set_sel_hi(-1);
            if let Some(first) = self.instances.first().map(|i| i.id.clone()) {
                self.select(ui, &first);
            } else {
                ui.set_instance_name("".into());
                ui.set_instance_state("—".into());
                ui.set_instance_code(instances::UNKNOWN);
            }
        } else {
            self.refresh_header(ui);
        }
    }

    /// Header fields come from the polled summary (name + combined_state).
    fn refresh_header(&self, ui: &App) {
        let Some(id) = self.selected.as_deref() else {
            return;
        };
        if let Some(summary) = self.instances.iter().find(|i| i.id == id) {
            let code = instances::state_code(&summary.combined_state);
            ui.set_instance_name(
                if summary.name.is_empty() {
                    summary.id.as_str()
                } else {
                    summary.name.as_str()
                }
                .into(),
            );
            ui.set_instance_state(instances::state_label(code).into());
            ui.set_instance_code(code);
        }
    }

    /// Accept at most `budget` lines into the model (batch, not per line).
    fn drain_ring(&mut self, budget: usize) {
        let mut left = budget;
        while left > 0 {
            let Some((instance, dto)) = self.pending.pop_front() else {
                break;
            };
            if self.selected.as_deref() != Some(instance.as_str()) {
                continue;
            }
            let line = console::make_line(
                dto.seq,
                console::format_ts(&dto.ts),
                &dto.line,
            );
            match self.ring.push(line) {
                PushOutcome::Accepted { evicted } => {
                    if evicted {
                        self.line_model.remove(0);
                    }
                    let line = self.ring.iter().last().expect("line just pushed");
                    self.line_model.push(LogLine {
                        seq: line.seq as i32,
                        ts: line.ts.as_str().into(),
                        tag: line.tag.as_str().into(),
                        level: line.level,
                        msg: line.msg.as_str().into(),
                    });
                    left -= 1;
                }
                PushOutcome::Duplicate => {}
            }
        }
    }

    fn copy(&mut self, lo: i32, hi: i32) {
        let n = self.ring.len() as i32;
        let (lo, hi) = (lo.max(0), hi.min(n - 1));
        if lo > hi || n == 0 {
            return;
        }
        let text: String = self
            .ring
            .iter()
            .skip(lo as usize)
            .take((hi - lo + 1) as usize)
            .map(|l| l.display())
            .collect::<Vec<_>>()
            .join("\n");
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        let preview: String = text.chars().take(60).collect();
        match self.clipboard.as_mut().map(|c| c.set_text(text.clone())) {
            Some(Ok(())) => eprintln!(
                "COPY ok lines={} chars={} first={preview}",
                hi - lo + 1,
                text.len()
            ),
            other => eprintln!("COPY FAILED {:?}", other.map(|r| r.err())),
        }
    }
}

fn main() -> Result<(), slint::PlatformError> {
    eprintln!("START minewrap-gui");

    // --- daemon discovery + auto-spawn (blocking, bounded ~11 s) ---
    let config_path = daemon::default_config_path();
    let settings = daemon::load_settings(&config_path);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let startup = daemon::ensure_daemon(&runtime, &config_path, &settings);
    let startup_note = match &startup {
        daemon::Startup::AlreadyRunning => "daemon already running".to_string(),
        daemon::Startup::Spawned => "daemon started".to_string(),
        daemon::Startup::Unavailable(e) => format!("daemon unavailable: {e}"),
    };
    eprintln!(
        "minewrap-gui: {startup_note} ({}:{} · data {})",
        settings.host, settings.port, settings.data_dir
    );

    let token = daemon::read_token(&settings.data_dir);
    if token.is_empty() {
        eprintln!(
            "minewrap-gui: no token at {}/token — API calls will be 401",
            settings.data_dir
        );
    }
    let client = RemoteClient::new(daemon::base_url(&settings), token);

    // --- UI + models ---
    let ui = App::new()?;
    let line_model = Rc::new(VecModel::from(Vec::<LogLine>::new()));
    let inst_model = Rc::new(VecModel::from(Vec::<InstRow>::new()));
    ui.set_lines(line_model.clone().into());
    ui.set_instances(inst_model.clone().into());
    ui.set_daemon_ok(false);
    ui.set_daemon_info("connecting…".into());
    ui.set_status(startup_note.clone().into());

    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (sel_tx, sel_rx) = watch::channel(None::<String>);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<UiMsg>();

    let state = Rc::new(RefCell::new(State {
        ring: Ring::new(),
        pending: VecDeque::new(),
        instances: Vec::new(),
        selected: None,
        line_model: line_model.clone(),
        inst_model: inst_model.clone(),
        cmd_tx,
        sel_tx,
        clipboard: None,
    }));

    runtime::spawn(runtime, client, sel_rx, cmd_rx, msg_tx);

    let ui_weak = ui.as_weak();
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_pick(move |idx| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            let idx = idx.max(0) as usize;
            if let Some(summary) = st.instances.get(idx) {
                let id = summary.id.clone();
                st.select(&ui, &id);
            }
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_step(move |delta| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            let n = st.instances.len();
            if n == 0 {
                return;
            }
            let current = st
                .selected
                .as_deref()
                .and_then(|id| st.instances.iter().position(|i| i.id == id))
                .unwrap_or(0) as i64;
            let next = (current + delta as i64).rem_euclid(n as i64) as usize;
            let id = st.instances[next].id.clone();
            st.select(&ui, &id);
        });
    }
    {
        let state = state.clone();
        ui.on_control(move |action| {
            let st = state.borrow();
            st.cmd_tx.send(Cmd::Control(action.to_string())).ok();
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_send_command(move |text| {
            let text = text.trim().to_string();
            if text.is_empty() {
                return;
            }
            let st = state.borrow();
            st.cmd_tx.send(Cmd::SendCommand(text)).ok();
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_cmd_text("".into());
            }
        });
    }
    {
        let state = state.clone();
        ui.on_copy_selection(move |lo, hi| {
            state.borrow_mut().copy(lo, hi);
        });
    }
    {
        let ui_weak = ui_weak.clone();
        ui.on_add_instance(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_status("Add instance wizard: chunk 2".into());
            }
        });
    }

    // --- 50 ms drain: messages + batched model pushes (400 lines/tick) ---
    let drain = {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        let timer = Timer::default();
        timer.start(TimerMode::Repeated, Duration::from_millis(50), move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            let mut messages = Vec::new();
            while let Ok(msg) = msg_rx.try_recv() {
                messages.push(msg);
            }
            for msg in messages {
                match msg {
                    UiMsg::Instances(list) => st.apply_instances(&ui, list),
                    UiMsg::Health(Ok(health)) => {
                        ui.set_daemon_ok(true);
                        ui.set_daemon_info(
                            format!(
                                "daemon {} · api {}",
                                if health.daemon_version.is_empty() {
                                    "?"
                                } else {
                                    health.daemon_version.as_str()
                                },
                                health.api_version
                            )
                            .into(),
                        );
                    }
                    UiMsg::Health(Err(_)) => {
                        ui.set_daemon_ok(false);
                        ui.set_daemon_info("daemon offline".into());
                    }
                    UiMsg::WsState(live) => ui.set_ws_live(live),
                    UiMsg::Lines { instance, lines } => {
                        if st.selected.as_deref() == Some(instance.as_str()) {
                            for line in lines {
                                st.pending.push_back((instance.clone(), line));
                            }
                            // Keep only the newest ~5 bursts if the UI lags.
                            while st.pending.len() > 5_000 {
                                st.pending.pop_front();
                            }
                        }
                    }
                    UiMsg::Notice(text) | UiMsg::Error(text) => ui.set_status(text.into()),
                }
            }
            st.drain_ring(400);
        });
        timer
    };

    ui.show()?;
    let result = slint::run_event_loop();
    drop(drain);
    eprintln!("EXIT minewrap-gui");
    result
}
