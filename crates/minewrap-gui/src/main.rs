//! minewrap-gui — pure-Rust Slint client for `minewrapd`.
//!
//! P1 chunk 4 (on top of chunks 1–2): multi-node. A `NodeService`
//! holds the local node +
//! the remote registry (`nodes.json`, mode 0600, XDG config dir). The
//! runtime thread polls `/health` + `GET /api/v1/instances` of every
//! node every 2 s and pushes `(node, state, instances)` to the UI.
//! Every instance-scoped REST call carries `node_id`, so it reaches the
//! right daemon; the console WebSocket connects only for the selected
//! instance (hence only for its node).
//!
//! P1 chunk 2 (add-instance wizard, per-instance Settings panel, Remove
//! instance) is unchanged, except every command is tagged with the node
//! of the selected instance.
//!
//! SPDX-License-Identifier: AGPL-3.0-only

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use minewrap_remote::types::{InstanceSummaryDto, LogLineDto};
use minewrap_gui::config_view::{DiscordForm, RAM_MIN_MB, SettingsForm, WizardForm};
use minewrap_gui::nodes::{self, NodeInstances, NodeService, LOCAL_NODE_ID};
use minewrap_gui::runtime::{self, Cmd, UiMsg};
use slint::{ModelRc, SharedString, Timer, TimerMode, VecModel};
use tokio::sync::{mpsc, watch};

use minewrap_gui::console::{self, PushOutcome, Ring};
use minewrap_gui::daemon;
use minewrap_gui::instances;

slint::include_modules!();

/// A line accepted by the ring but not yet pushed to the Slint model.
type PendingLine = (String, LogLineDto);

/// UI-thread state shared by callbacks and the 50 ms drain timer.
struct State {
    ring: Ring,
    pending: VecDeque<PendingLine>,
    /// `instance_id` → instances, grouped by the node that owns them.
    instances_by_node: HashMap<String, Vec<InstanceSummaryDto>>,
    selected: Option<String>,      // instance id shown in the main column
    selected_node: Option<String>, // node owning `selected`
    line_model: Rc<VecModel<LogLine>>,
    /// Rust mirror of the slint `nodes` model (sidebar groups).
    nodes: Vec<NodeRow>,
    node_model: Rc<VecModel<NodeRow>>,
    cmd_tx: mpsc::UnboundedSender<Cmd>,
    sel_tx: watch::Sender<Option<(String, String)>>,
    clipboard: Option<arboard::Clipboard>,
    /// The add-instance wizard dialog (created hidden, shown on demand).
    wizard: Option<WizardWindow>,
    /// The nodes window (Add/Remove remote-node table; lazy-created).
    nodes_window: Option<NodesWindow>,
    /// Dirty tracking for the Settings panel: the JSON snapshot of the
    /// form as loaded, compared against the live form on every edit.
    settings_saved: String,
    settings_dirty: bool,
    /// `properties-text` as last loaded; dirty when the live editor differs.
    properties_saved: String,
    properties_dirty: bool,
    /// Discord form as last loaded; dirty when the live form differs.
    notif_saved: String,
    notif_dirty: bool,
    /// Which panel a `GetConfig` response is routed to ("settings" |
    /// "notifications").
    config_target: Option<String>,
    /// Instance id the user tried to switch to while a panel is dirty
    /// (awaiting confirm-discard).
    pending_switch: Option<String>,
    /// Tab the user tried to open while the current panel is dirty
    /// (awaiting confirm-discard).
    pending_tab: Option<i32>,
    /// Name of the mod currently being toggled (per-row busy state).
    mods_toggled_name: String,
    /// Which widget triggered the last `scan-workdir` so its response can
    /// be routed back ("wizard" | "settings").
    scan_target: Option<String>,
    /// Cached `GET /api/v1/java` results, shared by wizard and settings.
    java_runtimes: Vec<minewrap_remote::types::JavaRuntimeDto>,
    /// Mods list for the selected instance (raw DTOs), plus the live
    /// search filter.
    mods: Vec<ModRow>,
    mods_filter: String,
}

impl State {
    /// Switch the console to `id` on `node_id`, resetting the
    /// ring/backfill cursor. The watch selection carries `(node, instance)`
    /// so the runtime reconnects the console WebSocket to the right node.
    fn select(&mut self, ui: &App, node_id: &str, id: &str) {
        if self.selected.as_deref() == Some(id)
            && self.selected_node.as_deref() == Some(node_id)
        {
            return;
        }
        self.selected_node = Some(node_id.to_string());
        self.selected = Some(id.to_string());
        self.ring.reset();
        self.pending.clear();
        self.line_model.set_vec(Vec::new());
        self.sel_tx
            .send(Some((node_id.to_string(), id.to_string())))
            .ok();
        ui.set_selected_id(id.into());
        ui.set_selected_node(node_id.into());
        ui.set_sel_lo(-1);
        ui.set_sel_hi(-1);
        ui.set_stick(true);
        self.refresh_header(ui);
        // The panel matching the current tab shows the selected instance:
        // (re)load its data; other panels drop their dirty state.
        match ui.get_settings_tab() {
            1 => load_settings(ui, self, node_id, id),
            2 => load_properties(ui, self, node_id, id),
            3 => load_mods(ui, self, node_id, id),
            4 => load_notifications(ui, self, node_id, id),
            _ => {
                self.settings_dirty = false;
                self.properties_dirty = false;
                self.notif_dirty = false;
                self.settings_saved.clear();
                self.properties_saved.clear();
                self.notif_saved.clear();
                ui.set_settings_dirty(false);
                ui.set_properties_dirty(false);
                ui.set_notif_dirty(false);
                ui.set_settings_status("".into());
                ui.set_properties_status("".into());
                ui.set_notif_status("".into());
            }
        }
    }

    /// Is any of the editable panels dirty?
    fn panel_dirty(&self) -> bool {
        self.settings_dirty || self.properties_dirty || self.notif_dirty
    }

    /// Clicked a node header in the sidebar: highlight the node and, when
    /// nothing is selected yet, select its first instance.
    fn pick_node(&mut self, ui: &App, id: &str) {
        self.selected_node = Some(id.to_string());
        ui.set_selected_node(id.into());
        if self.selected.is_none() {
            let first_id = self
                .instances_by_node
                .get(id)
                .and_then(|list| list.first().map(|i| i.id.clone()));
            if let Some(first_id) = first_id {
                self.select(ui, id, &first_id);
            }
        }
    }

    /// Rebuild the sidebar node model, keeping (or choosing) a valid
    /// selection.
    fn apply_instances(&mut self, ui: &App, all: Vec<NodeInstances>) {
        self.instances_by_node = all
            .iter()
            .map(|ni| (ni.node.id.clone(), ni.instances.clone()))
            .collect();

        let rows: Vec<NodeRow> = all
            .iter()
            .map(|ni| {
                let (label, code) = nodes::node_state_summary(ni.node.state);
                let inst_rows: Vec<InstRow> = ni
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
                NodeRow {
                    id: ni.node.id.as_str().into(),
                    name: ni.node.name.as_str().into(),
                    url: ni.node.url.as_str().into(),
                    is_local: ni.node.is_local,
                    state: label.into(),
                    code,
                    instances: Rc::new(VecModel::from(inst_rows)).into(),
                }
            })
            .collect();
        self.node_model.set_vec(rows.clone());
        self.nodes = rows;

        let still_valid = self.selected.as_ref().is_some_and(|id| {
            self.selected_node.as_deref().is_some_and(|node| {
                self.instances_by_node
                    .get(node)
                    .is_some_and(|list| list.iter().any(|i| &i.id == id))
            })
        });
        if !still_valid {
            self.pending.clear();
            self.ring.reset();
            self.line_model.set_vec(Vec::new());
            self.selected = None;
            self.sel_tx.send(None).ok();
            ui.set_selected_id("".into());
            ui.set_selected_node("".into());
            ui.set_sel_lo(-1);
            ui.set_sel_hi(-1);
            // Reselect: prefer the previously selected node, else the
            // first node that has instances (local node sorts first).
            let selected_node = self.selected_node.clone();
            if let Some(node) = selected_node.as_deref() {
                let first_id = self
                    .instances_by_node
                    .get(node)
                    .and_then(|list| list.first().map(|i| i.id.clone()));
                if let Some(first_id) = first_id {
                    self.select(ui, node, &first_id);
                    return;
                }
            }
            for ni in &all {
                let first_id = ni
                    .instances
                    .first()
                    .map(|i| i.id.clone());
                if let Some(first_id) = first_id {
                    self.select(ui, &ni.node.id, &first_id);
                    return;
                }
            }
            self.settings_dirty = false;
            self.properties_dirty = false;
            self.notif_dirty = false;
            self.settings_saved.clear();
            self.properties_saved.clear();
            self.notif_saved.clear();
            ui.set_settings_dirty(false);
            ui.set_properties_dirty(false);
            ui.set_notif_dirty(false);
            ui.set_instance_name("".into());
            ui.set_instance_state("—".into());
            ui.set_instance_code(instances::UNKNOWN);
        } else {
            self.refresh_header(ui);
        }
    }

    /// Header fields come from the polled summary (name + combined_state).
    fn refresh_header(&self, ui: &App) {
        let Some(node) = self.selected_node.as_deref() else {
            return;
        };
        let Some(id) = self.selected.as_deref() else {
            return;
        };
        let Some(summary) = self
            .instances_by_node
            .get(node)
            .and_then(|list| list.iter().find(|i| &i.id == id))
        else {
            return;
        };
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

/// Which node owns an instance id (from the latest poll).
fn resolve_node(state: &State, id: &str) -> Option<String> {
    state
        .instances_by_node
        .iter()
        .find(|(_, list)| list.iter().any(|i| &i.id == id))
        .map(|(node, _)| node.clone())
}

/// The `(node_id, instance_id)` of the current selection.
fn selection(state: &State) -> Option<(String, String)> {
    Some((
        state.selected_node.clone()?,
        state.selected.clone()?,
    ))
}

/// Title/text/action for the confirm dialog of the dirty panel (priority:
/// Settings > Properties > Notifications).
fn dirty_panel(state: &State) -> (&'static str, &'static str, &'static str) {
    if state.settings_dirty {
        (
            "Discard unsaved changes?",
            "The Settings panel has unsaved changes. Switch instance anyway?",
            "discard-settings",
        )
    } else if state.properties_dirty {
        (
            "Discard unsaved changes?",
            "server.properties has unsaved changes. Switch instance anyway?",
            "discard-properties",
        )
    } else {
        (
            "Discard unsaved changes?",
            "The Notifications panel has unsaved changes. Switch instance anyway?",
            "discard-notifications",
        )
    }
}

/// Apply a pending instance switch or tab change after a discard was
/// confirmed.
fn finish_pending(state: &mut State, ui: &App) {
    ui.set_confirm_open(false);
    if let Some(id) = state.pending_switch.take() {
        let node = match resolve_node(state, &id) {
            Some(n) => n,
            None => return,
        };
        state.select(ui, &node, &id);
        return;
    }
    if let Some(target) = state.pending_tab.take() {
        ui.set_settings_tab(target);
        match target {
            1 => {
                if let Some((node, id)) = selection(state) {
                    load_settings(ui, state, &node, &id);
                }
            }
            2 => {
                if let Some((node, id)) = selection(state) {
                    load_properties(ui, state, &node, &id);
                }
            }
            3 => {
                if let Some((node, id)) = selection(state) {
                    load_mods(ui, state, &node, &id);
                }
            }
            4 => {
                if let Some((node, id)) = selection(state) {
                    load_notifications(ui, state, &node, &id);
                }
            }
            _ => {}
        }
    }
}

/// Recompute the Properties dirty flag from the live editor vs the
/// loaded snapshot.
fn refresh_properties_dirty(ui: &App, state: &mut State) {
    let dirty = !state.properties_saved.is_empty()
        && ui.get_properties_text() != state.properties_saved;
    if dirty != state.properties_dirty {
        state.properties_dirty = dirty;
        ui.set_properties_dirty(dirty);
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

/// Load `GET …/config` for `id` on `node_id` into the Settings form.
fn load_settings(ui: &App, state: &mut State, node_id: &str, id: &str) {
    state.settings_dirty = false;
    state.settings_saved.clear();
    ui.set_settings_dirty(false);
    ui.set_settings_error("".into());
    ui.set_settings_status("".into());
    ui.set_settings_loading(true);
    state
        .cmd_tx
        .send(Cmd::GetConfig(node_id.to_string(), id.to_string()))
        .ok();
}

/// Load `GET …/properties` for `id` on `node_id` into the Properties panel.
fn load_properties(ui: &App, state: &mut State, node_id: &str, id: &str) {
    state.properties_saved.clear();
    ui.set_properties_dirty(false);
    ui.set_properties_error("".into());
    ui.set_properties_status("".into());
    ui.set_properties_loading(true);
    state
        .cmd_tx
        .send(Cmd::GetProperties(node_id.to_string(), id.to_string()))
        .ok();
}

/// Load `GET …/mods` for `id` on `node_id` into the Mods panel.
fn load_mods(ui: &App, state: &mut State, node_id: &str, id: &str) {
    ui.set_mods_status("".into());
    ui.set_mods_loading(true);
    state
        .cmd_tx
        .send(Cmd::ListMods(node_id.to_string(), id.to_string()))
        .ok();
}

/// Load `GET …/config` (discord group) for `id` on `node_id` into the Notifications panel.
fn load_notifications(ui: &App, state: &mut State, node_id: &str, id: &str) {
    state.config_target = Some("notifications".into());
    ui.set_notif_load_error("".into());
    ui.set_notif_error("".into());
    ui.set_notif_status("".into());
    state
        .cmd_tx
        .send(Cmd::GetConfig(node_id.to_string(), id.to_string()))
        .ok();
}

/// Read the live Notifications form from Slint properties.
fn read_notif_form(ui: &App) -> DiscordForm {
    let parse_f64 = |s: &str, def: f64| s.parse().unwrap_or(def);
    let parse_usize = |s: &str, def: usize| s.parse().unwrap_or(def);
    let redact: Vec<String> = ui
        .get_notif_redact()
        .split('\n')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let webhook = ui.get_notif_webhook().trim().to_string();
    DiscordForm {
        enabled: ui.get_notif_enabled(),
        mode: ui.get_notif_mode().to_string(),
        interval_seconds: parse_f64(&ui.get_notif_interval(), 600.0),
        first_push_delay_seconds: parse_f64(&ui.get_notif_first_push(), 10.0),
        log_lines: parse_usize(&ui.get_notif_log_lines(), 15),
        max_log_chars: parse_usize(&ui.get_notif_max_log_chars(), 1500),
        state_change_posts: ui.get_notif_state_change_posts(),
        state_change_debounce_seconds: parse_f64(&ui.get_notif_state_change_debounce(), 30.0),
        redact_regexes: redact,
        webhook_url: if webhook.is_empty() {
            None
        } else {
            Some(webhook)
        },
    }
}

/// Recompute the Notifications dirty flag from the live form vs the loaded snapshot.
fn refresh_notif_dirty(ui: &App, state: &mut State) {
    let dirty = !state.notif_saved.is_empty()
        && read_notif_form(ui).snapshot() != state.notif_saved;
    if dirty != state.notif_dirty {
        state.notif_dirty = dirty;
        ui.set_notif_dirty(dirty);
    }
}

/// Apply `GET …/config` to the Notifications panel.
fn apply_notifications(ui: &App, state: &mut State, value: &serde_json::Value) {
    let form = DiscordForm::from_view(value);
    state.notif_saved = form.snapshot();
    let discord = value.get("discord").and_then(|v| v.as_object());
    ui.set_notif_enabled(form.enabled);
    ui.set_notif_mode(form.mode.into());
    ui.set_notif_interval(form.interval_seconds.to_string().into());
    ui.set_notif_first_push(form.first_push_delay_seconds.to_string().into());
    ui.set_notif_log_lines(form.log_lines.to_string().into());
    ui.set_notif_max_log_chars(form.max_log_chars.to_string().into());
    ui.set_notif_state_change_posts(form.state_change_posts);
    ui.set_notif_state_change_debounce(
        form.state_change_debounce_seconds.to_string().into(),
    );
    ui.set_notif_redact(form.redact_regexes.join("\n").into());
    ui.set_notif_webhook_set(
        discord
            .and_then(|d| d.get("webhook_set"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    );
    ui.set_notif_webhook("".into());
    ui.set_notif_result("".into());
    ui.set_notif_result_ok(false);
    state.notif_dirty = false;
    ui.set_notif_dirty(false);
    ui.set_notif_load_error("".into());
    ui.set_notif_error("".into());
    ui.set_notif_status("".into());
    ui.set_notif_loaded(true);
}

/// Recompute the filtered mods model from the raw list + live filter.
fn recompute_mods(state: &mut State, ui: &App) {
    let filter = state.mods_filter.to_lowercase();
    let rows: Vec<ModRow> = state
        .mods
        .iter()
        .filter(|m| filter.is_empty() || m.name.to_lowercase().contains(&filter))
        .cloned()
        .collect();
    let enabled = rows.iter().filter(|m| m.enabled).count();
    ui.set_mods_count(format!("{} mods · {} enabled", rows.len(), enabled).into());
    ui.set_mods_empty(rows.is_empty());
    let model: ModelRc<ModRow> = Rc::new(VecModel::from(rows)).into();
    ui.set_mods_filtered(model);
}

/// Human-readable size (mirrors `minewrap_core::mods::format_size`).
fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{:.1} {}", size, UNITS[unit])
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
            st.cmd_tx
                .send(Cmd::ScanWorkdir(LOCAL_NODE_ID.to_string(), dir))
                .ok();
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
                .instances_by_node
                .values()
                .flatten()
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

/// Create (once) the nodes window: the Add/Remove remote-node panel.
fn ensure_nodes_window(state: &Rc<RefCell<State>>, ui: &App) {
    if state.borrow().nodes_window.is_some() {
        return;
    }
    let window = NodesWindow::new().expect("create nodes window");
    let ui_weak = ui.as_weak();
    {
        let state = state.clone();
        window.on_add_node(move |name, url, token| {
            let _ = state
                .borrow_mut()
                .cmd_tx
                .send(Cmd::AddNode {
                    name: name.to_string(),
                    url: url.to_string(),
                    token: token.to_string(),
                });
            let st = state.borrow();
            if let Some(w) = st.nodes_window.as_ref() {
                w.set_new_name("".into());
                w.set_new_url("".into());
                w.set_new_token("".into());
                w.set_nodes_error("".into());
            }
        });
    }
    {
        window.on_remove_node(move |id| {
            let id = id.to_string();
            let Some(ui) = ui_weak.upgrade() else { return };
            ui.set_confirm_title(format!("Remove node '{id}'?").into());
            ui.set_confirm_text(
                "Removing a node forgets its registry entry. The daemon itself keeps running. Continue?"
                    .into(),
            );
            ui.set_confirm_action("remove-node".into());
            ui.set_confirm_id(id.into());
            ui.set_confirm_open(true);
        });
    }
    {
        let state = state.clone();
        window.on_retry_node(move |id| {
            let _ = state
                .borrow_mut()
                .cmd_tx
                .send(Cmd::ProbeNode(id.to_string()));
        });
    }
    state.borrow_mut().nodes_window = Some(window);
}

fn open_nodes_window(state: &Rc<RefCell<State>>, ui: &App) {
    ensure_nodes_window(state, ui);
    let result = {
        let st = state.borrow();
        let model: ModelRc<NodeRow> = st.node_model.clone().into();
        let win = st.nodes_window.as_ref().expect("nodes window exists");
        win.set_nodes(model);
        win.set_nodes_error("".into());
        win.show()
    };
    if let Err(e) = result {
        eprintln!("minewrap-gui: show nodes window: {e}");
    }
}

fn main() -> Result<(), slint::PlatformError> {
    eprintln!("START minewrap-gui");

    // --- daemon discovery + auto-spawn (blocking, bounded ~11 s) ---
    let config_path = daemon::default_config_path();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let svc = Arc::new(
        NodeService::load(&config_path.to_string_lossy())
            .map_err(|e| slint::PlatformError::Other(e.to_string()))?,
    );
    let startup = daemon::ensure_daemon(&runtime, &config_path, &svc.local);
    let startup_note = match &startup {
        daemon::Startup::AlreadyRunning => "daemon already running".to_string(),
        daemon::Startup::Spawned => "daemon started".to_string(),
        daemon::Startup::Unavailable(e) => format!("daemon unavailable: {e}"),
    };
    eprintln!(
        "minewrap-gui: {startup_note} ({}:{} · data {})",
        svc.local.host, svc.local.port, svc.local.data_dir
    );

    let token = daemon::read_token(&svc.local.data_dir);
    if token.is_empty() {
        eprintln!(
            "minewrap-gui: no token at {}/token — API calls will be 401",
            svc.local.data_dir
        );
    }

    // --- UI + models ---
    let ui = App::new()?;
    let line_model = Rc::new(VecModel::from(Vec::<LogLine>::new()));
    let node_model: Rc<VecModel<NodeRow>> = Rc::new(VecModel::<NodeRow>::default());
    ui.set_lines(line_model.clone().into());
    ui.set_nodes(node_model.clone().into());
    ui.set_daemon_ok(false);
    ui.set_daemon_info("connecting…".into());
    ui.set_status(startup_note.clone().into());

    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (sel_tx, sel_rx) = watch::channel(None::<(String, String)>);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<UiMsg>();

    let state = Rc::new(RefCell::new(State {
        ring: Ring::new(),
        pending: VecDeque::new(),
        instances_by_node: HashMap::new(),
        selected: None,
        selected_node: None,
        line_model: line_model.clone(),
        nodes: Vec::new(),
        node_model,
        cmd_tx,
        sel_tx,
        clipboard: None,
        wizard: None,
        nodes_window: None,
        settings_saved: String::new(),
        settings_dirty: false,
        properties_saved: String::new(),
        properties_dirty: false,
        notif_saved: String::new(),
        notif_dirty: false,
        config_target: None,
        pending_switch: None,
        pending_tab: None,
        mods_toggled_name: String::new(),
        scan_target: None,
        java_runtimes: Vec::new(),
        mods: Vec::new(),
        mods_filter: String::new(),
    }));

    runtime::spawn(runtime, svc, sel_rx, cmd_rx, msg_tx);

    let ui_weak = ui.as_weak();
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_pick_node(move |id| {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            let id = id.to_string();
            let mut st = state.borrow_mut();
            st.pick_node(&ui, &id);
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_pick_inst(move |id| {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            let id = id.to_string();
            let mut st = state.borrow_mut();
            if st.panel_dirty() {
                st.pending_switch = Some(id.clone());
                let (title, text, action) = dirty_panel(&st);
                ui.set_confirm_title(title.into());
                ui.set_confirm_text(text.into());
                ui.set_confirm_action(action.into());
                ui.set_confirm_open(true);
                return;
            }
            let node = match resolve_node(&st, &id) {
                Some(n) => n,
                None => return,
            };
            st.select(&ui, &node, &id);
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
            let flat: Vec<String> = st
                .instances_by_node
                .values()
                .flatten()
                .map(|i| i.id.clone())
                .collect();
            let n = flat.len();
            if n == 0 {
                return;
            }
            let current = st
                .selected
                .as_deref()
                .and_then(|id| flat.iter().position(|x| x == id))
                .unwrap_or(0) as i64;
            let next = (current + delta as i64).rem_euclid(n as i64) as usize;
            let id = flat[next].clone();
            if st.panel_dirty() {
                st.pending_switch = Some(id.clone());
                let (title, text, action) = dirty_panel(&st);
                ui.set_confirm_title(title.into());
                ui.set_confirm_text(text.into());
                ui.set_confirm_action(action.into());
                ui.set_confirm_open(true);
                return;
            }
            let node = match resolve_node(&st, &id) {
                Some(n) => n,
                None => return,
            };
            st.select(&ui, &node, &id);
        });
    }
    {
        let state = state.clone();
        ui.on_control(move |action| {
            let state = state.clone();
            let st = state.borrow();
            let id = st.selected.as_deref().unwrap_or("").to_string();
            let node = st.selected_node.as_deref().unwrap_or("").to_string();
            drop(st);
            if id.is_empty() {
                return;
            }
            state
                .borrow_mut()
                .cmd_tx
                .send(Cmd::Control(node, id, action.to_string()))
                .ok();
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
            let id = st.selected.as_deref().unwrap_or("").to_string();
            let node = st.selected_node.as_deref().unwrap_or("").to_string();
            drop(st);
            if id.is_empty() {
                return;
            }
            state
                .borrow_mut()
                .cmd_tx
                .send(Cmd::SendCommand(node, id, text))
                .ok();
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
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_add_node(move || {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            open_nodes_window(&state, &ui);
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_remove_instance(move |id| {
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            let state = state.clone();
            let st = state.borrow();
            let node = st.selected_node.as_deref().unwrap_or("").to_string();
            drop(st);
            ui.set_confirm_title(format!("Remove instance '{id}'?").into());
            ui.set_confirm_text(
                "The server process will be stopped and the instance deleted. This cannot be undone."
                    .into(),
            );
            ui.set_confirm_action("remove".into());
            ui.set_confirm_id(id.into());
            ui.set_confirm_node(node.into());
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
            let node = st.selected_node.as_deref().unwrap_or("").to_string();
            drop(st);
            if id.is_empty() {
                return;
            }
            let form = read_settings_form(&ui);
            if let Err(e) = form.validate() {
                ui.set_settings_error(e.into());
                return;
            }
            let patch = form.to_patch();
            state
                .borrow_mut()
                .cmd_tx
                .send(Cmd::UpdateConfig(node, id, patch))
                .ok();
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
            if let Some((node, id)) = selection(&st) {
                load_settings(&ui, &mut st, &node, &id);
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
            let node = st.selected_node.as_deref().unwrap_or("").to_string();
            drop(st);
            state
                .borrow_mut()
                .cmd_tx
                .send(Cmd::ScanWorkdir(node, dir))
                .ok();
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_properties_touched(move || {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            refresh_properties_dirty(&ui, &mut st);
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_properties_save(move || {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            let st = state.borrow();
            let id = st.selected.as_deref().unwrap_or("").to_string();
            let node = st.selected_node.as_deref().unwrap_or("").to_string();
            drop(st);
            if id.is_empty() {
                return;
            }
            let st = state.borrow_mut();
            let text = ui.get_properties_text().to_string();
            st.cmd_tx
                .send(Cmd::WriteProperties(node, id, text))
                .ok();
            ui.set_properties_loading(true);
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_properties_revert(move || {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            if let Some((node, id)) = selection(&st) {
                load_properties(&ui, &mut st, &node, &id);
            }
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_mods_touched(move || {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            st.mods_filter = ui.get_mods_filter().to_string();
            recompute_mods(&mut st, &ui);
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_mods_toggle(move |name, _enabled| {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            let name = name.to_string();
            let mut st = state.borrow_mut();
            let id = st.selected.as_deref().unwrap_or("").to_string();
            let node = st.selected_node.as_deref().unwrap_or("").to_string();
            if id.is_empty() {
                return;
            }
            st.mods_toggled_name = name.clone();
            ui.set_mods_busy(name.clone().into());
            let current = st.mods.iter().find(|m| m.name == name).map(|m| m.enabled).unwrap_or(false);
            st.cmd_tx
                .send(Cmd::ToggleMod(node, id, name, !current))
                .ok();
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_notif_touched(move || {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            refresh_notif_dirty(&ui, &mut st);
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_notif_save(move || {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            let st = state.borrow();
            let id = st.selected.as_deref().unwrap_or("").to_string();
            let node = st.selected_node.as_deref().unwrap_or("").to_string();
            drop(st);
            if id.is_empty() {
                return;
            }
            let form = read_notif_form(&ui);
            if let Err(e) = form.validate() {
                ui.set_notif_error(e.into());
                return;
            }
            let st = state.borrow_mut();
            let patch = serde_json::json!({ "discord": form.to_patch() });
            st.cmd_tx
                .send(Cmd::NotifSave(node, id, patch))
                .ok();
            ui.set_notif_saving(true);
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_notif_revert(move || {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            if let Some((node, id)) = selection(&st) {
                load_notifications(&ui, &mut st, &node, &id);
            }
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_notif_test(move || {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            let st = state.borrow();
            let id = st.selected.as_deref().unwrap_or("").to_string();
            let node = st.selected_node.as_deref().unwrap_or("").to_string();
            drop(st);
            if id.is_empty() {
                return;
            }
            let st = state.borrow_mut();
            st.cmd_tx.send(Cmd::DiscordTest(node, id)).ok();
            ui.set_notif_sending(true);
            ui.set_notif_result("".into());
        });
    }
    {
        let state = state.clone();
        let ui_weak = ui_weak.clone();
        ui.on_navigate_tab(move |target| {
            let state = state.clone();
            let ui_weak = ui_weak.clone();
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            let current = ui.get_settings_tab();
            // Leaving a dirty panel needs confirmation first.
            if current != target && st.panel_dirty() {
                st.pending_tab = Some(target);
                let (title, text, action) = match current {
                    1 => (
                        "Discard unsaved changes?",
                        "The Settings panel has unsaved changes. Leave anyway?",
                        "discard-settings",
                    ),
                    2 => (
                        "Discard unsaved changes?",
                        "server.properties has unsaved changes. Leave anyway?",
                        "discard-properties",
                    ),
                    4 => (
                        "Discard unsaved changes?",
                        "The Notifications panel has unsaved changes. Leave anyway?",
                        "discard-notifications",
                    ),
                    // Console (0) and Mods (3) are never dirty.
                    _ => return,
                };
                ui.set_confirm_title(title.into());
                ui.set_confirm_text(text.into());
                ui.set_confirm_action(action.into());
                ui.set_confirm_open(true);
                return;
            }
            ui.set_settings_tab(target);
            match target {
                1 => {
                    if st.settings_dirty {
                        st.pending_tab = Some(1);
                        ui.set_confirm_title("Discard unsaved changes?".into());
                        ui.set_confirm_text(
                            "The Settings panel has unsaved changes. Continue anyway?"
                                .into(),
                        );
                        ui.set_confirm_action("discard-settings".into());
                        ui.set_confirm_open(true);
                        return;
                    }
                    if let Some((node, id)) = selection(&st) {
                        load_settings(&ui, &mut st, &node, &id);
                    }
                }
                2 => {
                    if let Some((node, id)) = selection(&st) {
                        load_properties(&ui, &mut st, &node, &id);
                    }
                }
                3 => {
                    if let Some((node, id)) = selection(&st) {
                        load_mods(&ui, &mut st, &node, &id);
                    }
                }
                4 => {
                    if let Some((node, id)) = selection(&st) {
                        load_notifications(&ui, &mut st, &node, &id);
                    }
                }
                _ => {}
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
                    let node = ui.get_confirm_node().to_string();
                    st.cmd_tx
                        .send(Cmd::RemoveInstance(node, target))
                        .ok();
                }
                "remove-node" => {
                    st.cmd_tx.send(Cmd::RemoveNode(target)).ok();
                }
                "discard-settings" => {
                    st.settings_dirty = false;
                    st.settings_saved.clear();
                    ui.set_settings_dirty(false);
                    ui.set_settings_error("".into());
                    ui.set_settings_status("".into());
                    finish_pending(&mut st, &ui);
                }
                "discard-properties" => {
                    st.properties_dirty = false;
                    ui.set_properties_dirty(false);
                    ui.set_properties_status("".into());
                    finish_pending(&mut st, &ui);
                }
                "discard-notifications" => {
                    st.notif_dirty = false;
                    ui.set_notif_dirty(false);
                    ui.set_notif_status("".into());
                    finish_pending(&mut st, &ui);
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
                let is_error = matches!(&msg, UiMsg::Error(_));
                match msg {
                    UiMsg::Instances(list) => st.apply_instances(&ui, list),
                    UiMsg::NodeState(node_id, state_) => {
                        if let Some(pos) = st.nodes.iter().position(|n| n.id == node_id) {
                            let (label, code) = nodes::node_state_summary(state_);
                            let mut row = st.nodes[pos].clone();
                            row.state = label.into();
                            row.code = code;
                            st.nodes[pos] = row;
                            st.node_model.set_vec(st.nodes.clone());
                        }
                    }
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
                    UiMsg::Notice(text) | UiMsg::Error(text) => {
                        ui.set_status(text.clone().into());
                        if let Some(win) = st.nodes_window.as_ref() {
                            if is_error {
                                win.set_nodes_error(text.clone().into());
                            } else {
                                win.set_nodes_error("".into());
                            }
                        }
                    }
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
                        match st
                            .config_target
                            .take()
                            .unwrap_or_default()
                            .as_str()
                        {
                            "notifications" => {
                                apply_notifications(&ui, &mut st, &value);
                            }
                            _ => apply_settings(&ui, &mut st, &value),
                        }
                    }
                    UiMsg::ConfigFetched(Err(e)) => {
                        match st
                            .config_target
                            .take()
                            .unwrap_or_default()
                            .as_str()
                        {
                            "notifications" => {
                                ui.set_notif_load_error(e.into());
                            }
                            _ => {
                                ui.set_settings_error(e.into());
                                ui.set_settings_loading(false);
                            }
                        }
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
                    UiMsg::PropertiesFetched(Ok(value)) => {
                        let text = value
                            .get("text")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let errors: Vec<String> = value
                            .get("errors")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|e| e.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default();
                        let err_text = if errors.is_empty() {
                            "".into()
                        } else {
                            errors.join(" · ")
                        };
                        st.properties_saved = text.clone();
                        ui.set_properties_text(text.into());
                        ui.set_properties_error(err_text.into());
                        st.properties_dirty = false;
                        ui.set_properties_dirty(false);
                        ui.set_properties_loading(false);
                    }
                    UiMsg::PropertiesFetched(Err(e)) => {
                        let friendly = if e.contains("not found") {
                            "No server.properties in work dir.".into()
                        } else {
                            e.into()
                        };
                        ui.set_properties_error(friendly);
                        ui.set_properties_loading(false);
                    }
                    UiMsg::PropertiesSaved(Ok(())) => {
                        st.properties_saved = ui.get_properties_text().to_string();
                        st.properties_dirty = false;
                        ui.set_properties_dirty(false);
                        ui.set_properties_error("".into());
                        ui.set_properties_loading(false);
                        ui.set_properties_status("Saved.".into());
                    }
                    UiMsg::PropertiesSaved(Err(e)) => {
                        ui.set_properties_error(e.into());
                        ui.set_properties_loading(false);
                    }
                    UiMsg::ModsFetched(Ok(list)) => {
                        let rows: Vec<ModRow> = list
                            .into_iter()
                            .map(|m| ModRow {
                                name: m.name.into(),
                                size: format_size(m.size).into(),
                                enabled: m.enabled,
                            })
                            .collect();
                        st.mods = rows.clone();
                        recompute_mods(&mut st, &ui);
                        ui.set_mods_loading(false);
                    }
                    UiMsg::ModsFetched(Err(e)) => {
                        ui.set_mods_status(e.into());
                        ui.set_mods_loading(false);
                    }
                    UiMsg::ModsToggled(Ok(enabled)) => {
                        let name = std::mem::take(&mut st.mods_toggled_name);
                        let what = if enabled { "enabled" } else { "disabled" };
                        ui.set_mods_status(format!("{name} {what}.").into());
                        if let Some(id) = st.selected.clone() {
                            let node = st.selected_node.clone().unwrap_or_default();
                            st.cmd_tx.send(Cmd::ListMods(node, id)).ok();
                        }
                    }
                    UiMsg::ModsToggled(Err(e)) => {
                        st.mods_toggled_name.clear();
                        ui.set_mods_status(e.into());
                    }
                    UiMsg::NotifSaved(Ok(())) => {
                        let form = read_notif_form(&ui);
                        st.notif_saved = form.snapshot();
                        st.notif_dirty = false;
                        ui.set_notif_dirty(false);
                        ui.set_notif_saving(false);
                        ui.set_notif_error("".into());
                        ui.set_notif_status("Saved.".into());
                        ui.set_notif_webhook("".into());
                    }
                    UiMsg::NotifSaved(Err(e)) => {
                        ui.set_notif_saving(false);
                        ui.set_notif_error(e.into());
                    }
                    UiMsg::DiscordTestFetched(Ok(r)) => {
                        ui.set_notif_sending(false);
                        if r.ok {
                            ui.set_notif_result("Test sent.".into());
                            ui.set_notif_result_ok(true);
                        } else {
                            ui.set_notif_result(
                                format!("Failed: {}", r.error.unwrap_or_default()).into(),
                            );
                            ui.set_notif_result_ok(false);
                        }
                    }
                    UiMsg::DiscordTestFetched(Err(e)) => {
                        ui.set_notif_sending(false);
                        ui.set_notif_result(format!("Failed: {e}").into());
                        ui.set_notif_result_ok(false);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn properties_dirty(text: &str, saved: &str) -> bool {
        !saved.is_empty() && text != saved
    }

    #[test]
    fn properties_dirty_transition() {
        // Nothing loaded yet: never dirty.
        assert!(!properties_dirty("motd=P1C3", ""));
        // Loaded and unchanged: clean.
        assert!(!properties_dirty("motd=P1C3\n", "motd=P1C3\n"));
        // Edited: dirty.
        assert!(properties_dirty("motd=P1C3-edited\n", "motd=P1C3\n"));
        // Reverted back to the saved text: clean again.
        assert!(!properties_dirty("motd=P1C3\n", "motd=P1C3\n"));
    }

    #[test]
    fn mods_filter_is_case_insensitive() {
        let mods = vec![
            ModRow {
                name: "Alpha.jar".into(),
                size: "1.0 KiB".into(),
                enabled: true,
            },
            ModRow {
                name: "beta.jar".into(),
                size: "2.0 KiB".into(),
                enabled: false,
            },
            ModRow {
                name: "GAMMA.jar.disabled".into(),
                size: "3.0 KiB".into(),
                enabled: true,
            },
        ];
        let filter = |m: &ModRow, f: &str| {
            let f = f.to_lowercase();
            f.is_empty() || m.name.to_lowercase().contains(&f)
        };
        assert_eq!(mods.iter().filter(|m| filter(m, "ALP")).count(), 1);
        assert_eq!(mods.iter().filter(|m| filter(m, "alp")).count(), 1);
        assert_eq!(mods.iter().filter(|m| filter(m, "jar")).count(), 3);
        assert_eq!(
            mods.iter().filter(|m| filter(m, "jar.disabled")).count(),
            1
        );
        assert_eq!(mods.iter().filter(|m| filter(m, "")).count(), 3);
    }

    #[test]
    fn format_size_renders_units() {
        assert_eq!(format_size(0), "0.0 B");
        assert_eq!(format_size(500), "500.0 B");
        assert_eq!(format_size(1500), "1.5 KiB");
        assert_eq!(format_size(1_500_000), "1.4 MiB");
        assert_eq!(format_size(1_500_000_000), "1.4 GiB");
    }
}
