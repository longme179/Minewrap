//! minewrap-gui — pure-Rust Slint client for `minewrapd`.
//!
//! P1 chunk 2 (on top of chunk 1): add-instance wizard dialog, per-instance
//! Settings panel (Instance / Probe / Restart, dirty tracking, discard
//! confirmation), Remove instance with confirmation, and the REST calls they
//! need (add / remove / get-config / update-config / detect-java /
//! scan-workdir) routed through the same runtime thread + mpsc channel as
//! chunk 1 — no blocking of the Slint UI thread.
//!
//! SPDX-License-Identifier: AGPL-3.0-only

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use minewrap_remote::RemoteClient;
use minewrap_remote::types::{InstanceSummaryDto, LogLineDto};
use minewrap_gui::config_view::{RAM_MIN_MB, SettingsForm, WizardForm};
use slint::{ModelRc, SharedString, Timer, TimerMode, VecModel};
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
    /// The add-instance wizard dialog (created hidden, shown on demand).
    wizard: Option<WizardWindow>,
    /// Dirty tracking for the Settings panel: the JSON snapshot of the
    /// form as loaded, compared against the live form on every edit.
    settings_saved: String,
    settings_dirty: bool,
    /// Instance id the user tried to switch to while the Settings panel is
    /// dirty (awaiting confirm-discard).
    pending_switch: Option<String>,
    /// Which widget triggered the last `scan-workdir` so its response can
    /// be routed back ("wizard" | "settings").
    scan_target: Option<String>,
    /// Cached `GET /api/v1/java` results, shared by wizard and settings.
    java_runtimes: Vec<minewrap_remote::types::JavaRuntimeDto>,
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
        // The Settings panel shows the selected instance: (re)load its config.
        if ui.get_settings_tab() == 1 {
            load_settings(ui, self, id);
        } else {
            self.settings_dirty = false;
            self.settings_saved.clear();
            ui.set_settings_dirty(false);
        }
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
            self.settings_dirty = false;
            self.settings_saved.clear();
            ui.set_settings_dirty(false);
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

/// Combo box items for the Java picker: "java" (system default), every
/// detected runtime binary, then "Custom path…".
fn java_items(state: &State) -> Rc<VecModel<SharedString>> {
    let mut items: Vec<SharedString> = vec!["java".into()];
    for rt in &state.java_runtimes {
        items.push(rt.binary.clone().into());
    }
    items.push("Custom path…".into());
    Rc::new(VecModel::from(items))
}

/// `java_items` plus `current` at the front when missing, so a configured
/// java_path that matches no detected runtime still shows (Slint clears a
/// ComboBox selection whose current-value is not in the model).
fn java_items_with(state: &State, current: &str) -> Rc<VecModel<SharedString>> {
    let mut items: Vec<SharedString> = vec!["java".into()];
    for rt in &state.java_runtimes {
        items.push(rt.binary.clone().into());
    }
    items.push("Custom path…".into());
    if !current.is_empty() && !items.iter().any(|s| s.as_str() == current) {
        items.insert(0, current.into());
    }
    Rc::new(VecModel::from(items))
}

/// Load `GET …/config` for `id` into the Settings form.
fn load_settings(ui: &App, state: &mut State, id: &str) {
    state.settings_dirty = false;
    state.settings_saved.clear();
    ui.set_settings_dirty(false);
    ui.set_settings_error("".into());
    ui.set_settings_status("".into());
    ui.set_settings_loading(true);
    state.cmd_tx.send(Cmd::GetConfig(id.to_string())).ok();
}

/// Read the live Settings form from Slint properties.
fn read_settings_form(ui: &App) -> SettingsForm {
    let parse_u16 = |s: &str, def: u16| s.parse().unwrap_or(def);
    let parse_u32 = |s: &str, def: u32| s.parse().unwrap_or(def);
    let parse_f64 = |s: &str, def: f64| s.parse().unwrap_or(def);
    SettingsForm {
        id: ui.get_selected_id().to_string(),
        name: ui.get_settings_name().to_string(),
        work_dir: ui.get_settings_work_dir().to_string(),
        java_path: ui.get_settings_java().to_string(),
        min_ram_mb: parse_u32(&ui.get_settings_min_ram(), RAM_MIN_MB),
        max_ram_mb: parse_u32(&ui.get_settings_max_ram(), RAM_MIN_MB),
        preset: ui.get_settings_preset().to_string(),
        jvm_extra: ui.get_settings_jvm_extra().to_string(),
        launch_mode: ui.get_settings_launch_mode().to_string(),
        launch_args: ui.get_settings_launch_args().to_string(),
        autostart: ui.get_settings_autostart(),
        probe_host: ui.get_settings_probe_host().to_string(),
        probe_port: parse_u16(&ui.get_settings_probe_port(), 25565),
        probe_query_enabled: ui.get_settings_probe_query(),
        probe_timeout: parse_f64(&ui.get_settings_probe_timeout(), 3.0),
        restart_policy: ui.get_settings_restart_policy().to_string(),
        restart_max_retries: parse_u32(&ui.get_settings_restart_max_retries(), 5),
        restart_backoff_base: parse_f64(&ui.get_settings_restart_backoff_base(), 5.0),
        restart_backoff_max: parse_f64(&ui.get_settings_restart_backoff_max(), 300.0),
        restart_reset_after: parse_f64(&ui.get_settings_restart_reset_after(), 600.0),
    }
}

/// Recompute the dirty flag from the live form vs the loaded snapshot.
fn refresh_dirty(ui: &App, state: &mut State) {
    let dirty = !state.settings_saved.is_empty()
        && read_settings_form(ui).snapshot() != state.settings_saved;
    if dirty != state.settings_dirty {
        state.settings_dirty = dirty;
        ui.set_settings_dirty(dirty);
    }
}

/// Open (or re-open) the add-instance wizard with default values.
fn ensure_wizard(state: &Rc<RefCell<State>>) {
    if state.borrow().wizard.is_some() {
        return;
    }
    let wizard = WizardWindow::new().expect("create wizard window");
    // --- wizard dialog callbacks (capture `state` via clones) ---
    {
        let state = state.clone();
        wizard.on_cancel(move || {
            let state = state.clone();
            let st = state.borrow();
            if let Some(wiz) = st.wizard.as_ref() {
                let _ = wiz.hide();
                wiz.set_wiz_error("".into());
            }
        });
    }
    {
        let state = state.clone();
        wizard.on_scan_workdir(move || {
            let state = state.clone();
            // Scope the immutable borrow: a live `borrow()` followed by
            // `borrow_mut()` in the same scope panics ("RefCell already
            // borrowed") — seen live when Scan was clicked.
            let dir = {
                let st = state.borrow();
                match st.wizard.as_ref() {
                    Some(wiz) => wiz.get_wiz_work_dir().trim().to_string(),
                    None => return,
                }
            };
            if dir.is_empty() {
                let st = state.borrow();
                if let Some(wiz) = st.wizard.as_ref() {
                    wiz.set_wiz_error("Enter a work dir first.".into());
                }
                return;
            }
            let mut st = state.borrow_mut();
            st.scan_target = Some("wizard".into());
            if let Some(wiz) = st.wizard.as_ref() {
                wiz.set_wiz_busy(true);
                wiz.set_wiz_error("".into());
                wiz.set_wiz_scan_note("".into());
            }
            st.cmd_tx.send(Cmd::ScanWorkdir(dir)).ok();
        });
    }
    {
        let state = state.clone();
        wizard.on_detect_java(move || {
            let state = state.clone();
            state.borrow_mut().cmd_tx.send(Cmd::DetectJava).ok();
        });
    }
    {
        let state = state.clone();
        wizard.on_submit(move || {
            let state = state.clone();
            let st = state.borrow();
            let wiz = st.wizard.as_ref().unwrap();
            let id = wiz.get_wiz_id().trim().to_string();
            let name = wiz.get_wiz_name().trim().to_string();
            let work_dir = wiz.get_wiz_work_dir().trim().to_string();
            let java_path = wiz.get_wiz_java().trim().to_string();
            let min_ram: u32 = wiz.get_wiz_min_ram().parse().unwrap_or(0);
            let max_ram: u32 = wiz.get_wiz_max_ram().parse().unwrap_or(0);
            let preset = wiz.get_wiz_preset().to_string();
            let launch_mode = wiz.get_wiz_launch_mode().to_string();
            let launch_args = wiz.get_wiz_launch_args().to_string();
            let autostart = wiz.get_wiz_autostart();
            let _ = wiz;
            let existing: Vec<String> = st
                .instances
                .iter()
                .map(|i| i.id.clone())
                .collect();
            let form = WizardForm {
                id,
                name,
                work_dir,
                java_path,
                min_ram_mb: min_ram,
                max_ram_mb: max_ram,
                preset,
                launch_mode,
                launch_args,
                autostart,
            };
            if let Err(e) = form.validate(&existing) {
                if let Some(wiz) = st.wizard.as_ref() {
                    wiz.set_wiz_error(e.into());
                }
                return;
            }
            let payload = form.to_create_payload();
            if let Some(wiz) = st.wizard.as_ref() {
                wiz.set_wiz_busy(true);
                wiz.set_wiz_error("".into());
            }
            st.cmd_tx.send(Cmd::AddInstance(payload)).ok();
        });
    }
    // Move the wizard into State; closures above capture `state` only.
    state.borrow_mut().wizard = Some(wizard);
}

fn open_wizard(state: &Rc<RefCell<State>>) {
    // The wizard window is created lazily here: a window created hidden
    // before the event loop starts never commits its first buffer on the
    // winit-wayland + software-renderer path (verified live on Slint 1.18),
    // so it must be created after `run()` and shown from a callback.
    ensure_wizard(state);
    let st = state.borrow_mut();
    let model: ModelRc<SharedString> = {
        let items = java_items(&st);
        items.into()
    };
    let wiz = st.wizard.as_ref().unwrap();
    wiz.set_wiz_java_items(model.clone());
    wiz.set_wiz_id("".into());
    wiz.set_wiz_name("".into());
    wiz.set_wiz_work_dir("".into());
    wiz.set_wiz_java("java".into());
    wiz.set_wiz_min_ram(RAM_MIN_MB.to_string().into());
    wiz.set_wiz_max_ram((RAM_MIN_MB * 2).to_string().into());
    wiz.set_wiz_preset("none".into());
    wiz.set_wiz_launch_mode("jar".into());
    wiz.set_wiz_launch_args("".into());
    wiz.set_wiz_autostart(false);
    wiz.set_wiz_java_custom("".into());
    wiz.set_wiz_error("".into());
    wiz.set_wiz_scan_note("".into());
    wiz.set_wiz_busy(false);
    wiz.set_wiz_step(1);
    if let Err(e) = wiz.show() {
        eprintln!("minewrap-gui: show wizard: {e}");
    }
    if st.java_runtimes.is_empty() {
        st.cmd_tx.send(Cmd::DetectJava).ok();
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
        wizard: None,
        settings_saved: String::new(),
        settings_dirty: false,
        pending_switch: None,
        scan_target: None,
        java_runtimes: Vec::new(),
    }));

    runtime::spawn(runtime, client, sel_rx, cmd_rx, msg_tx);

    let ui_weak = ui.as_weak();
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_pick(move |idx| {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            let idx = idx.max(0) as usize;
            let target = st.instances.get(idx).map(|i| i.id.clone());
            if st.settings_dirty {
                st.pending_switch = target.clone();
                ui.set_confirm_title("Discard unsaved changes?".into());
                ui.set_confirm_text(
                    "The Settings panel has unsaved changes. Switch instance anyway?"
                        .into(),
                );
                ui.set_confirm_action("discard-settings".into());
                ui.set_confirm_open(true);
                return;
            }
            if let Some(id) = target {
                st.select(&ui, &id);
            }
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_step(move |delta| {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
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
            let target = st.instances[next].id.clone();
            if st.settings_dirty {
                st.pending_switch = Some(target.clone());
                ui.set_confirm_title("Discard unsaved changes?".into());
                ui.set_confirm_text(
                    "The Settings panel has unsaved changes. Switch instance anyway?"
                        .into(),
                );
                ui.set_confirm_action("discard-settings".into());
                ui.set_confirm_open(true);
                return;
            }
            st.select(&ui, &target);
        });
    }
    {
        let state = state.clone();
        ui.on_control(move |action| {
            let state = state.clone();
            let st = state.borrow();
            st.cmd_tx.send(Cmd::Control(action.to_string())).ok();
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_send_command(move |text| {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            let text = text.trim().to_string();
            if text.is_empty() {
                return;
            }
            let st = state.borrow();
            st.cmd_tx.send(Cmd::SendCommand(text)).ok();
            ui.set_cmd_text("".into());
        });
    }
    {
        let state = state.clone();
        ui.on_copy_selection(move |lo, hi| {
            let state = state.clone();
            state.borrow_mut().copy(lo, hi);
        });
    }
    {
        let state = state.clone();
        ui.on_add_instance(move || {
            let state = state.clone();
            open_wizard(&state);
        });
    }
    {
        let ui_weak = ui_weak.clone();
        ui.on_remove_instance(move |id| {
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            ui.set_confirm_title(format!("Remove instance '{id}'?").into());
            ui.set_confirm_text(
                "The server process will be stopped and the instance deleted. This cannot be undone."
                    .into(),
            );
            ui.set_confirm_action("remove".into());
            ui.set_confirm_id(id.into());
            ui.set_confirm_open(true);
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_settings_touched(move || {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            refresh_dirty(&ui, &mut st);
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_settings_save(move || {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            let st = state.borrow();
            let id = st.selected.as_deref().unwrap_or("").to_string();
            if id.is_empty() {
                return;
            }
            let form = read_settings_form(&ui);
            if let Err(e) = form.validate() {
                ui.set_settings_error(e.into());
                return;
            }
            let patch = form.to_patch();
            st.cmd_tx.send(Cmd::UpdateConfig(id, patch)).ok();
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_settings_revert(move || {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            if let Some(id) = st.selected.clone() {
                load_settings(&ui, &mut st, &id);
            }
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_settings_scan_workdir(move || {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            let dir = ui.get_settings_work_dir().trim().to_string();
            if dir.is_empty() {
                return;
            }
            let mut st = state.borrow_mut();
            st.scan_target = Some("settings".into());
            ui.set_settings_scan_note("".into());
            ui.set_settings_error("".into());
            st.cmd_tx.send(Cmd::ScanWorkdir(dir)).ok();
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_settings_tab_activated(move || {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            if ui.get_settings_tab() != 1 {
                return;
            }
            let mut st = state.borrow_mut();
            if st.settings_dirty {
                ui.set_confirm_title("Discard unsaved changes?".into());
                ui.set_confirm_text(
                    "The Settings panel has unsaved changes. Continue anyway?"
                        .into(),
                );
                ui.set_confirm_action("discard-settings".into());
                ui.set_confirm_open(true);
                return;
            }
            if let Some(id) = st.selected.clone() {
                load_settings(&ui, &mut st, &id);
            }
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_confirm_yes(move || {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            ui.set_confirm_open(false);
            let action = ui.get_confirm_action().to_string();
            let target = ui.get_confirm_id().to_string();
            match action.as_str() {
                "remove" => {
                    st.cmd_tx.send(Cmd::RemoveInstance(target)).ok();
                }
                "discard-settings" => {
                    st.settings_dirty = false;
                    st.settings_saved.clear();
                    ui.set_settings_dirty(false);
                    ui.set_settings_error("".into());
                    if let Some(id) = st.pending_switch.take() {
                        st.select(&ui, &id);
                    }
                }
                _ => {}
            }
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_confirm_no(move || {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
            let mut st = state.borrow_mut();
            st.pending_switch = None;
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_confirm_open(false);
            }
        });
    }

    // --- 50 ms drain: messages + batched model pushes (400 lines/tick) ---
    let drain = {
        let state = state.clone();
        let ui_weak = ui.as_weak();
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
                    UiMsg::InstanceAdded(Ok(())) => {
                        if let Some(wiz) = &st.wizard {
                            let _ = wiz.hide();
                        }
                        ui.set_status("Instance added.".into());
                    }
                    UiMsg::InstanceAdded(Err(e)) => {
                        if let Some(wiz) = &st.wizard {
                            wiz.set_wiz_busy(false);
                            wiz.set_wiz_error(e.into());
                        }
                    }
                    UiMsg::InstanceRemoved(Ok(_id)) => {
                        ui.set_status("Instance removed.".into());
                    }
                    UiMsg::InstanceRemoved(Err(e)) => {
                        ui.set_status(format!("remove instance: {e}").into());
                    }
                    UiMsg::ConfigFetched(Ok(value)) => {
                        apply_settings(&ui, &mut st, &value);
                    }
                    UiMsg::ConfigFetched(Err(e)) => {
                        ui.set_settings_error(e.into());
                        ui.set_settings_loading(false);
                    }
                    UiMsg::ConfigSaved(Ok(())) => {
                        st.settings_saved = read_settings_form(&ui).snapshot();
                        st.settings_dirty = false;
                        ui.set_settings_dirty(false);
                        ui.set_settings_error("".into());
                        ui.set_settings_status("Saved.".into());
                    }
                    UiMsg::ConfigSaved(Err(e)) => {
                        ui.set_settings_error(e.into());
                        ui.set_settings_loading(false);
                    }
                    UiMsg::JavaRuntimes(Ok(list)) => {
                        st.java_runtimes = list;
                        let current = ui.get_settings_java().to_string();
                        let model: ModelRc<SharedString> = {
                            let items = java_items_with(&st, &current);
                            items.into()
                        };
                        ui.set_settings_java_items(model.clone());
                        if let Some(wiz) = &st.wizard {
                            wiz.set_wiz_java_items(model);
                        }
                    }
                    UiMsg::JavaRuntimes(Err(e)) => {
                        eprintln!("minewrap-gui: detect java: {e}");
                    }
                    UiMsg::WorkDirScan(Ok(value)) => {
                        let scan = parse_scan(&value);
                        match st.scan_target.as_deref() {
                            Some("wizard") => {
                                if let Some(wiz) = &st.wizard {
                                    wiz.set_wiz_scan_note(scan.note.into());
                                    if let Some(args) = scan.launch_args {
                                        wiz.set_wiz_launch_args(args.into());
                                    }
                                }
                            }
                            Some("settings") => {
                                ui.set_settings_scan_note(scan.note.into());
                            }
                            _ => {}
                        }
                        st.scan_target = None;
                        if let Some(wiz) = &st.wizard {
                            wiz.set_wiz_busy(false);
                        }
                    }
                    UiMsg::WorkDirScan(Err(e)) => {
                        match st.scan_target.as_deref() {
                            Some("wizard") => {
                                if let Some(wiz) = &st.wizard {
                                    wiz.set_wiz_busy(false);
                                    wiz.set_wiz_error(e.into());
                                }
                            }
                            Some("settings") => {
                                ui.set_settings_error(e.into());
                                ui.set_settings_loading(false);
                            }
                            _ => {}
                        }
                        st.scan_target = None;
                    }
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

/// `GET …/config` → Settings form fields.
fn apply_settings(ui: &App, state: &mut State, value: &serde_json::Value) {
    let form = SettingsForm::from_view(value);
    state.settings_saved = form.snapshot();
    let items = java_items_with(state, &form.java_path);
    ui.set_settings_java_items(items.into());
    ui.set_settings_name(form.name.into());
    ui.set_settings_work_dir(form.work_dir.into());
    ui.set_settings_java(form.java_path.into());
    ui.set_settings_min_ram(form.min_ram_mb.to_string().into());
    ui.set_settings_max_ram(form.max_ram_mb.to_string().into());
    ui.set_settings_preset(form.preset.into());
    ui.set_settings_jvm_extra(form.jvm_extra.into());
    ui.set_settings_launch_mode(form.launch_mode.into());
    ui.set_settings_launch_args(form.launch_args.into());
    ui.set_settings_autostart(form.autostart);
    ui.set_settings_probe_host(form.probe_host.into());
    ui.set_settings_probe_port(form.probe_port.to_string().into());
    ui.set_settings_probe_query(form.probe_query_enabled);
    ui.set_settings_probe_timeout(form.probe_timeout.to_string().into());
    ui.set_settings_restart_policy(form.restart_policy.into());
    ui.set_settings_restart_max_retries(form.restart_max_retries.to_string().into());
    ui.set_settings_restart_backoff_base(form.restart_backoff_base.to_string().into());
    ui.set_settings_restart_backoff_max(form.restart_backoff_max.to_string().into());
    ui.set_settings_restart_reset_after(form.restart_reset_after.to_string().into());
    state.settings_dirty = false;
    ui.set_settings_dirty(false);
    ui.set_settings_error("".into());
    ui.set_settings_status("".into());
    ui.set_settings_loading(false);
}

/// Parse `POST /api/v1/scan-workdir` response into a human note plus an
/// optional launch-args prefill (first jar candidate's suggested args).
struct ScanResult {
    note: String,
    launch_args: Option<String>,
}

fn parse_scan(value: &serde_json::Value) -> ScanResult {
    let mut note = String::new();
    if let Some(report) = value.get("report").and_then(|v| v.as_object()) {
        let readable = report
            .get("readable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let writable = report
            .get("writable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let sp = report
            .get("has_server_properties")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let eula = report.get("has_eula").and_then(|v| v.as_bool()).unwrap_or(false);
        note = format!(
            "Readable: {} · Writable: {} · server.properties: {} · eula.txt: {}",
            yes_no(readable),
            yes_no(writable),
            yes_no(sp),
            yes_no(eula),
        );
        if let Some(notes) = report.get("notes").and_then(|v| v.as_array()) {
            for n in notes {
                if let Some(s) = n.as_str() {
                    note.push_str(" · ");
                    note.push_str(s);
                }
            }
        }
    }
    let mut launch_args = None;
    if let Some(cands) = value.get("candidates").and_then(|v| v.as_array()) {
        let mut picked: Option<&serde_json::Map<String, serde_json::Value>> = None;
        for c in cands {
            let obj = match c.as_object() {
                Some(o) => o,
                None => continue,
            };
            let kind = obj.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            if kind == "jar" {
                picked = Some(obj);
                break;
            }
        }
        if picked.is_none() {
            picked = cands.first().and_then(|c| c.as_object());
        }
        if let Some(obj) = picked {
            if let Some(args) = obj
                .get("suggested_launch_args")
                .and_then(|v| v.as_array())
            {
                launch_args = Some(
                    args.iter()
                        .filter_map(|a| a.as_str())
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
        }
    }
    ScanResult { note, launch_args }
}

fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}
