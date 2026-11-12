//! Form <-> wire payload mapping for the add-instance wizard and the
//! Settings panel, plus validation. Pure Rust (no Slint) so it can be
//! unit-tested without a UI toolkit.
//!
//! SPDX-License-Identifier: AGPL-3.0-only

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Aikar's Flags (https://aikar.co/flags.html) — the recommended JVM
/// preset for modern Minecraft servers. Mirror of
// `minewrap_core::config::aikars_flags`, duplicated here so
// minewrap-gui does not link minewrap-core. The leading `-Xms2G`/`-Xmx4G`
/// are intentionally included: callers that already add RAM args from the
/// form should use `aikar_flags_non_ram` instead.
pub const AIKAR_FLAGS: &[&str] = &[
    "-Xms2G",
    "-Xmx4G",
    "-XX:+UseG1GC",
    "-XX:+ParallelRefProcEnabled",
    "-XX:MaxGCPauseMillis=200",
    "-XX:+UnlockExperimentalVMOptions",
    "-XX:+DisableExplicitGC",
    "-XX:+AlwaysPreTouch",
    "-XX:G1NewSizeRatio=30",
    "-XX:G1MaxNewSizeRatio=40",
    "-XX:G1HeapRegionSize=8M",
    "-XX:G1ReservePercent=20",
    "-XX:G1HeapWastePercent=5",
    "-XX:G1MixedGCCountTarget=4",
    "-XX:InitiatingHeapOccupancyPercent=15",
    "-XX:G1MixedGCLiveThresholdPercent=90",
    "-XX:G1RSetUpdatingPauseTimePercent=5",
    "-XX:SurvivorRatio=32",
    "-XX:+PerfDisableSharedMem",
    "-XX:MaxTenuringThreshold=1",
    "-Dusing.aikars.flags=https://mcflags.env",
    "-Daikars.flags.file=aikars.flags",
    "-Dlog4j2.formatMsgNoLookups=true",
];

/// Allowed RAM range (MiB), mirrors the UI's number inputs.
pub const RAM_MIN_MB: u32 = 256;
pub const RAM_MAX_MB: u32 = 65536;

/// Default RAM (MiB) when the wire carries no `-Xms`/`-Xmx`.
pub const DEFAULT_MIN_RAM_MB: u32 = 2048;
pub const DEFAULT_MAX_RAM_MB: u32 = 4096;
pub const DEFAULT_PROBE_PORT: u16 = 25565;
pub const DEFAULT_PROBE_TIMEOUT: f64 = 3.0;

/// Matches v1 `INSTANCE_ID_RE`: `^[a-z0-9][a-z0-9_-]{0,62}[a-z0-9]$` — a
/// single alphanumeric character is allowed (mirrors
// `minewrap_core::config::valid_instance_id`).
pub fn valid_instance_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return false;
    }
    if !(bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit()) {
        return false;
    }
    if bytes.len() > 1
        && !(bytes[bytes.len() - 1].is_ascii_lowercase()
            || bytes[bytes.len() - 1].is_ascii_digit())
    {
        return false;
    }
    bytes.iter().all(|b| {
        b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_' || *b == b'-'
    })
}

/// Aikar flags without the `-Xms`/`-Xmx` entries — used when the form
/// already supplies RAM args from Min/Max RAM.
/// Aikar flags without the `-Xms`/`-Xmx` entries — used when the form
/// already supplies RAM args from Min/Max RAM.
pub fn aikar_flags_non_ram() -> Vec<&'static str> {
    AIKAR_FLAGS
        .iter()
        .filter(|a| parse_ram_arg(a).is_none())
        .copied()
        .collect()
}

/// `-Xms{N}G` when `mb` is a multiple of 1024, else `-Xms{N}M`.
pub fn ram_arg(mb: u32) -> String {
    if mb >= 1024 && mb % 1024 == 0 {
        format!("{}G", mb / 1024)
    } else {
        format!("{}M", mb)
    }
}

/// Parse `-Xms128M` / `-Xmx4G` / `-Xmx2048` into MiB.
pub fn parse_ram_arg(arg: &str) -> Option<u32> {
    let (prefix, rest) = arg
        .strip_prefix("-Xms")
        .map(|r| ("-Xms", r))
        .or_else(|| arg.strip_prefix("-Xmx").map(|r| ("-Xmx", r)))?;
    let (num_str, unit) = if rest.ends_with('M') || rest.ends_with('G') {
        let len = rest.len() - 1;
        (&rest[..len], &rest[len..])
    } else {
        (rest, "")
    };
    let mb: u32 = num_str.parse().ok()?;
    let mb = match unit {
        "G" => mb.checked_mul(1024)?,
        _ => mb,
    };
    let _ = prefix;
    Some(mb)
}

/// Split `jvm_args` into (min RAM, max RAM, extra args, preset).
/// Preset is `"aikar"` when any arg mentions aikar, else `"custom"` when
/// extra (non-RAM) args exist, else `"none"`.
fn split_jvm_args(args: &[String]) -> (u32, u32, String, String) {
    let mut min = DEFAULT_MIN_RAM_MB;
    let mut max = DEFAULT_MAX_RAM_MB;
    let mut extra: Vec<String> = Vec::new();
    let mut aikar = false;
    for arg in args {
        if let Some(mb) = parse_ram_arg(arg) {
            if arg.starts_with("-Xms") {
                min = mb;
            } else if arg.starts_with("-Xmx") {
                max = mb;
            }
        } else if arg.contains("aikar") {
            aikar = true;
            extra.push(arg.clone());
        } else {
            extra.push(arg.clone());
        }
    }
    let preset = if aikar {
        "aikar".to_string()
    } else if !extra.is_empty() {
        "custom".to_string()
    } else {
        "none".to_string()
    };
    (min, max, extra.join("\n"), preset)
}

/// Build the `jvm_args` array for a `POST /instances` body or a `PUT …/config`
/// patch from Min/Max RAM, preset, and extra args.
fn jvm_args_value(min_ram_mb: u32, max_ram_mb: u32, preset: &str, extra: &str) -> Vec<Value> {
    let mut jvm_args: Vec<Value> = vec![
        Value::String(format!("-Xms{}", ram_arg(min_ram_mb))),
        Value::String(format!("-Xmx{}", ram_arg(max_ram_mb))),
    ];
    if preset == "aikar" {
        jvm_args.extend(aikar_flags_non_ram().iter().map(|s| Value::String(s.to_string())));
    }
    if preset == "custom" {
        for line in extra.lines() {
            let line = line.trim();
            if line.is_empty() || parse_ram_arg(line).is_some() {
                continue;
            }
            jvm_args.push(Value::String(line.to_string()));
        }
    }
    jvm_args
}

/// Add-instance wizard form state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WizardForm {
    pub id: String,
    pub name: String,
    pub work_dir: String,
    pub java_path: String,
    pub min_ram_mb: u32,
    pub max_ram_mb: u32,
    pub preset: String,    // "none" | "aikar" | "custom"
    pub launch_mode: String, // "jar" | "args-file" | "custom" (UI only)
    pub launch_args: String,
    pub autostart: bool,
}

impl Default for WizardForm {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            work_dir: String::new(),
            java_path: "java".to_string(),
            min_ram_mb: DEFAULT_MIN_RAM_MB,
            max_ram_mb: DEFAULT_MAX_RAM_MB,
            preset: "none".to_string(),
            launch_mode: "jar".to_string(),
            launch_args: String::new(),
            autostart: false,
        }
    }
}

impl WizardForm {
    /// Validate before submit: id shape + not duplicate, work dir non-empty,
    /// RAM bounds and min <= max.
    pub fn validate(&self, existing_ids: &[String]) -> Result<(), String> {
        if !valid_instance_id(&self.id) {
            return Err(
                "ID must be lowercase letters/digits/_/-, 1–64 chars, and not \
                 start or end with - or _."
                    .to_string(),
            );
        }
        if existing_ids.iter().any(|existing| existing == &self.id) {
            return Err(format!("Instance '{}' already exists.", self.id));
        }
        if self.work_dir.trim().is_empty() {
            return Err("Work dir is required.".to_string());
        }
        for (label, mb) in [("Min RAM", self.min_ram_mb), ("Max RAM", self.max_ram_mb)] {
            if mb < RAM_MIN_MB || mb > RAM_MAX_MB {
                return Err(format!("{label} must be between 256 and 65536 MiB."));
            }
        }
        if self.max_ram_mb < self.min_ram_mb {
            return Err("Max RAM must be >= Min RAM.".to_string());
        }
        Ok(())
    }

    /// Build the `POST /api/v1/instances` body (wire shape of
    /// `InstanceConfig`; unknown fields are denied by the daemon, so only
    /// known keys are sent).
    /// JSON snapshot for dirty tracking.
    pub fn snapshot(&self) -> String {
        serde_json::to_string(self).unwrap()
    }

    /// `POST /api/v1/instances` body — the create payload doubles as the
    /// full configuration patch.
    pub fn to_patch(&self) -> Value {
        self.to_create_payload()
    }

    pub fn to_create_payload(&self) -> Value {
        let jvm_args = jvm_args_value(self.min_ram_mb, self.max_ram_mb, self.preset.as_str(), "");
        let mut map = serde_json::Map::new();
        map.insert("id".to_string(), Value::String(self.id.clone()));
        if !self.name.is_empty() {
            map.insert("name".to_string(), Value::String(self.name.clone()));
        }
        map.insert("work_dir".to_string(), Value::String(self.work_dir.clone()));
        if !self.java_path.is_empty() {
            map.insert(
                "java_path".to_string(),
                Value::String(self.java_path.clone()),
            );
        }
        map.insert("jvm_args".to_string(), Value::Array(jvm_args));
        let args: Vec<Value> = self
            .launch_args
            .split_whitespace()
            .map(|a| Value::String(a.to_string()))
            .collect();
        if !args.is_empty() {
            map.insert("launch_args".to_string(), Value::Array(args));
        }
        map.insert("autostart".to_string(), Value::Bool(self.autostart));
        Value::Object(map)
    }
}

/// Settings panel form state, mirroring `InstanceConfig`'s view shape
/// (`GET …/config` → `instance_view`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SettingsForm {
    pub id: String,
    pub name: String,
    pub work_dir: String,
    pub java_path: String,
    pub min_ram_mb: u32,
    pub max_ram_mb: u32,
    pub preset: String,
    pub jvm_extra: String, // one extra JVM arg per line
    pub launch_mode: String, // UI only
    pub launch_args: String,
    pub autostart: bool,
    pub probe_host: String,
    pub probe_port: u16,
    pub probe_query_enabled: bool,
    pub probe_timeout: f64,
    pub restart_policy: String,
    pub restart_max_retries: u32,
    pub restart_backoff_base: f64,
    pub restart_backoff_max: f64,
    pub restart_reset_after: f64,
}

impl SettingsForm {
    pub fn from_view(value: &Value) -> Self {
        let jvm_args: Vec<String> = value
            .get("jvm_args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let (min_ram, max_ram, jvm_extra, preset) = split_jvm_args(&jvm_args);

        let launch_args: Vec<String> = value
            .get("launch_args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let probe = value.get("probe").and_then(|v| v.as_object());
        let restart = value.get("restart").and_then(|v| v.as_object());

        Self {
            id: value
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            name: value.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            work_dir: value
                .get("work_dir")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            java_path: value
                .get("java_path")
                .and_then(|v| v.as_str())
                .unwrap_or("java")
                .to_string(),
            min_ram_mb: min_ram,
            max_ram_mb: max_ram,
            preset,
            jvm_extra,
            launch_mode: "jar".to_string(),
            launch_args: launch_args.join(" "),
            autostart: value.get("autostart").and_then(|v| v.as_bool()).unwrap_or(false),
            probe_host: probe
                .and_then(|p| p.get("host").and_then(|v| v.as_str()))
                .unwrap_or("127.0.0.1")
                .to_string(),
            probe_port: probe
                .and_then(|p| p.get("port").and_then(|v| v.as_u64()))
                .unwrap_or(DEFAULT_PROBE_PORT as u64) as u16,
            probe_query_enabled: probe
                .and_then(|p| p.get("query_enabled").and_then(|v| v.as_bool()))
                .unwrap_or(false),
            probe_timeout: probe
                .and_then(|p| p.get("timeout").and_then(|v| v.as_f64()))
                .unwrap_or(DEFAULT_PROBE_TIMEOUT),
            restart_policy: restart
                .and_then(|r| r.get("policy").and_then(|v| v.as_str()))
                .unwrap_or("on-crash")
                .to_string(),
            restart_max_retries: restart
                .and_then(|r| r.get("max_retries").and_then(|v| v.as_u64()))
                .unwrap_or(5) as u32,
            restart_backoff_base: restart
                .and_then(|r| r.get("backoff_base_seconds").and_then(|v| v.as_f64()))
                .unwrap_or(5.0),
            restart_backoff_max: restart
                .and_then(|r| r.get("backoff_max_seconds").and_then(|v| v.as_f64()))
                .unwrap_or(300.0),
            restart_reset_after: restart
                .and_then(|r| r.get("reset_after_online_seconds").and_then(|v| v.as_f64()))
                .unwrap_or(600.0),
        }
    }

    /// Build the `PUT …/config` patch (only known keys, so the daemon's
    /// `patch_instance` accepts it; absent keys leave the field untouched).
    pub fn to_patch(&self) -> Value {
        let mut jvm_args: Vec<Value> = vec![
            Value::String(format!("-Xms{}", ram_arg(self.min_ram_mb))),
            Value::String(format!("-Xmx{}", ram_arg(self.max_ram_mb))),
        ];
        match self.preset.as_str() {
            "aikar" => {
                jvm_args.extend(
                    aikar_flags_non_ram()
                        .iter()
                        .map(|s| Value::String(s.to_string())),
                );
            }
            "custom" => {
                for line in self.jvm_extra.lines() {
                    let line = line.trim();
                    if line.is_empty() || parse_ram_arg(line).is_some() {
                        continue;
                    }
                    jvm_args.push(Value::String(line.to_string()));
                }
            }
            _ => {}
        }

        let mut map = serde_json::Map::new();
        if !self.name.is_empty() {
            map.insert("name".to_string(), Value::String(self.name.clone()));
        }
        if !self.work_dir.is_empty() {
            map.insert("work_dir".to_string(), Value::String(self.work_dir.clone()));
        }
        if !self.java_path.is_empty() {
            map.insert(
                "java_path".to_string(),
                Value::String(self.java_path.clone()),
            );
        }
        map.insert("jvm_args".to_string(), Value::Array(jvm_args));
        let args: Vec<Value> = self
            .launch_args
            .split_whitespace()
            .map(|a| Value::String(a.to_string()))
            .collect();
        if !args.is_empty() {
            map.insert("launch_args".to_string(), Value::Array(args));
        }
        map.insert("autostart".to_string(), Value::Bool(self.autostart));
        map.insert(
            "probe".to_string(),
            Value::Object(serde_json::Map::from_iter([
                ("host".to_string(), Value::String(self.probe_host.clone())),
                (
                    "port".to_string(),
                    Value::Number(serde_json::Number::from(self.probe_port)),
                ),
                (
                    "query_enabled".to_string(),
                    Value::Bool(self.probe_query_enabled),
                ),
                (
                    "timeout".to_string(),
                    Value::Number(f64_num(self.probe_timeout)),
                ),
            ])),
        );
        map.insert(
            "restart".to_string(),
            Value::Object(serde_json::Map::from_iter([
                (
                    "policy".to_string(),
                    Value::String(self.restart_policy.clone()),
                ),
                (
                    "max_retries".to_string(),
                    Value::Number(serde_json::Number::from(self.restart_max_retries)),
                ),
                (
                    "backoff_base_seconds".to_string(),
                    Value::Number(f64_num(self.restart_backoff_base)),
                ),
                (
                    "backoff_max_seconds".to_string(),
                    Value::Number(f64_num(self.restart_backoff_max)),
                ),
                (
                    "reset_after_online_seconds".to_string(),
                    Value::Number(f64_num(self.restart_reset_after)),
                ),
            ])),
        );
        Value::Object(map)
    }

    /// JSON snapshot for dirty tracking.
    pub fn snapshot(&self) -> String {
        serde_json::to_string(self).unwrap()
    }

    pub fn validate(&self) -> Result<(), String> {
        for (label, mb) in [("Min RAM", self.min_ram_mb), ("Max RAM", self.max_ram_mb)] {
            if mb < RAM_MIN_MB || mb > RAM_MAX_MB {
                return Err(format!("{label} must be between 256 and 65536 MiB."));
            }
        }
        if self.max_ram_mb < self.min_ram_mb {
            return Err("Max RAM must be >= Min RAM.".to_string());
        }
        Ok(())
    }
    }


/// `serde_json::Number` from an `f64`, falling back to an integer when the
/// float is not representable (e.g. NaN) — keeps the patch JSON tidy.
fn f64_num(f: f64) -> serde_json::Number {
    serde_json::Number::from_f64(f).unwrap_or(serde_json::Number::from(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wizard_base() -> WizardForm {
        WizardForm {
            id: "survival".to_string(),
            name: "Survival".to_string(),
            work_dir: "/srv/minecraft/survival".to_string(),
            java_path: "/usr/lib/jvm/bin/java".to_string(),
            min_ram_mb: 1024,
            max_ram_mb: 8192,
            preset: "aikar".to_string(),
            launch_mode: "custom".to_string(),
            launch_args: "-jar server.jar nogui".to_string(),
            autostart: true,
        }
    }

    #[test]
    fn wizard_create_payload_has_all_known_fields() {
        let form = wizard_base();
        let p = form.to_create_payload();
        assert_eq!(p.get("id").and_then(|v| v.as_str()), Some("survival"));
        assert_eq!(p.get("name").and_then(|v| v.as_str()), Some("Survival"));
        assert_eq!(
            p.get("work_dir").and_then(|v| v.as_str()),
            Some("/srv/minecraft/survival")
        );
        assert_eq!(
            p.get("java_path").and_then(|v| v.as_str()),
            Some("/usr/lib/jvm/bin/java")
        );
        assert_eq!(p.get("autostart").and_then(|v| v.as_bool()), Some(true));
        let jvm = p.get("jvm_args").and_then(|v| v.as_array()).unwrap();
        assert_eq!(jvm[0], "-Xms1G");
        assert_eq!(jvm[1], "-Xmx8G");
        assert!(jvm.iter().any(|a| a == "-XX:+UseG1GC")); // aikar preset
        assert_eq!(jvm.len(), 2 + aikar_flags_non_ram().len());
        let args = p.get("launch_args").and_then(|v| v.as_array()).unwrap();
        assert_eq!(args, &["-jar", "server.jar", "nogui"]);
        // launch_mode is UI-only, never sent.
        assert!(!p.as_object().unwrap().contains_key("launch_mode"));
        assert!(!p.as_object().unwrap().contains_key("preset"));
    }

    #[test]
    fn wizard_create_payload_omits_optional_empty_fields() {
        let form = WizardForm {
            id: "a".to_string(),
            name: String::new(),
            work_dir: "/tmp/x".to_string(),
            java_path: String::new(),
            min_ram_mb: 256,
            max_ram_mb: 256,
            preset: "none".to_string(),
            launch_mode: "jar".to_string(),
            launch_args: String::new(),
            autostart: false,
        };
        let p = form.to_create_payload();
        let obj = p.as_object().unwrap();
        assert!(!obj.contains_key("name"));
        assert!(!obj.contains_key("java_path"));
        assert!(!obj.contains_key("launch_args"));
        assert_eq!(obj.get("jvm_args").and_then(|v| v.as_array()).unwrap().len(), 2);
    }

    #[test]
    fn wizard_validation_rejects_duplicate_and_bad_input() {
        let mut form = wizard_base();
        assert!(form.validate(&[]).is_ok());
        assert!(form.validate(&["survival".to_string()]).is_err()); // duplicate

        form.id = "trailing-".to_string();
        assert!(form.validate(&[]).is_err()); // bad id shape
        form.id = "ok".to_string();

        form.work_dir = "   ".to_string();
        assert!(form.validate(&[]).is_err()); // empty work dir
        form.work_dir = "/tmp/x".to_string();

        form.min_ram_mb = 100;
        assert!(form.validate(&[]).is_err()); // below min
        form.min_ram_mb = 256;
        form.max_ram_mb = 70000;
        assert!(form.validate(&[]).is_err()); // above max
        form.max_ram_mb = 200;
        assert!(form.validate(&[]).is_err()); // max < min
        form.max_ram_mb = 4096;
        assert!(form.validate(&[]).is_ok());
    }

    #[test]
    fn ram_arg_and_parse_round_trip() {
        assert_eq!(ram_arg(2048), "2G");
        assert_eq!(ram_arg(4096), "4G");
        assert_eq!(ram_arg(512), "512M");
        assert_eq!(ram_arg(3000), "3000M");
        assert_eq!(parse_ram_arg("-Xms2G"), Some(2048));
        assert_eq!(parse_ram_arg("-Xmx512M"), Some(512));
        assert_eq!(parse_ram_arg("-Xmx2048"), Some(2048));
        assert_eq!(parse_ram_arg("-Xms1G"), Some(1024));
        assert_eq!(parse_ram_arg("-XX:+UseG1GC"), None);
        assert_eq!(parse_ram_arg("bogus"), None);
    }

    #[test]
    fn valid_instance_id_matches_v1_rules() {
        assert!(valid_instance_id("survival"));
        assert!(valid_instance_id("a")); // single char allowed
        assert!(valid_instance_id("a-1_b"));
        assert!(!valid_instance_id(""));
        assert!(!valid_instance_id("trailing-"));
        assert!(!valid_instance_id("-leading"));
        assert!(!valid_instance_id("_leading"));
        assert!(!valid_instance_id("UPPER"));
        assert!(!valid_instance_id("has space"));
        assert!(!valid_instance_id(&"a".repeat(65)));
    }

    #[test]
    fn settings_from_view_round_trip() {
        let form = SettingsForm {
            id: "survival".into(),
            name: "Survival".into(),
            work_dir: "/srv/minecraft/survival".into(),
            java_path: "/usr/lib/jvm/bin/java".into(),
            min_ram_mb: 1024,
            max_ram_mb: 8192,
            preset: "custom".into(),
            jvm_extra: "-XX:+UseG1GC\n-XX:MaxGCPauseMillis=200".into(),
            launch_mode: "custom".into(),
            launch_args: "-jar server.jar nogui".into(),
            autostart: true,
            probe_host: "127.0.0.1".into(),
            probe_port: 25590,
            probe_query_enabled: true,
            probe_timeout: 5.5,
            restart_policy: "never".into(),
            restart_max_retries: 3,
            restart_backoff_base: 2.0,
            restart_backoff_max: 60.0,
            restart_reset_after: 120.0,
        };

        let view: Value = serde_json::from_value(form.to_patch()).unwrap();
        // Simulate the daemon echoing the view back (minimal instance_view
        // shape): add id/name/work_dir/java_path/autostart like the daemon
        // does, then parse.
        let echoed = serde_json::json!({
            "id": form.id,
            "name": form.name,
            "work_dir": form.work_dir,
            "java_path": form.java_path,
            "jvm_args": view.get("jvm_args"),
            "launch_args": view.get("launch_args"),
            "autostart": form.autostart,
            "probe": view.get("probe"),
            "restart": view.get("restart"),
        });
        let parsed = SettingsForm::from_view(&echoed);
        assert_eq!(parsed.name, form.name);
        assert_eq!(parsed.work_dir, form.work_dir);
        assert_eq!(parsed.java_path, form.java_path);
        assert_eq!(parsed.min_ram_mb, form.min_ram_mb);
        assert_eq!(parsed.max_ram_mb, form.max_ram_mb);
        assert_eq!(parsed.preset, form.preset);
        assert_eq!(parsed.jvm_extra, form.jvm_extra);
        assert_eq!(parsed.launch_args, form.launch_args);
        assert_eq!(parsed.autostart, form.autostart);
        assert_eq!(parsed.probe_host, form.probe_host);
        assert_eq!(parsed.probe_port, form.probe_port);
        assert_eq!(parsed.probe_query_enabled, form.probe_query_enabled);
        assert_eq!((parsed.probe_timeout * 10.0) as u32, (form.probe_timeout * 10.0) as u32);
        assert_eq!(parsed.restart_policy, form.restart_policy);
        assert_eq!(parsed.restart_max_retries, form.restart_max_retries);
        assert_eq!((parsed.restart_backoff_base * 10.0) as u32,
            (form.restart_backoff_base * 10.0) as u32);
        assert_eq!((parsed.restart_backoff_max * 10.0) as u32,
            (form.restart_backoff_max * 10.0) as u32);
        assert_eq!((parsed.restart_reset_after * 10.0) as u32,
            (form.restart_reset_after * 10.0) as u32);
    }

    #[test]
    fn settings_patch_contains_only_known_keys() {
        let form = SettingsForm {
            id: "survival".into(),
            name: "Survival".into(),
            work_dir: "/srv/minecraft/survival".into(),
            java_path: "/usr/lib/jvm/bin/java".into(),
            min_ram_mb: 1024,
            max_ram_mb: 8192,
            preset: "custom".into(),
            jvm_extra: "-XX:+UseG1GC\n-XX:MaxGCPauseMillis=200".into(),
            launch_mode: "custom".into(),
            launch_args: "-jar server.jar nogui".into(),
            autostart: true,
            probe_host: "127.0.0.1".into(),
            probe_port: 25565,
            probe_query_enabled: true,
            probe_timeout: 5.5,
            restart_policy: "never".into(),
            restart_max_retries: 3,
            restart_backoff_base: 2.0,
            restart_backoff_max: 60.0,
            restart_reset_after: 120.0,
        };
        let p = form.to_patch();
        let obj = p.as_object().unwrap();
        for key in ["name", "work_dir", "java_path", "jvm_args", "launch_args",
                    "autostart", "probe", "restart"] {
            assert!(obj.contains_key(key), "missing {key}");
        }
        assert!(!obj.contains_key("preset"));
        assert!(!obj.contains_key("launch_mode"));
        assert!(!obj.contains_key("jvm_extra"));
        // probe/restart are fully populated objects.
        assert_eq!(p.get("probe").and_then(|v| v.get("port")).and_then(|v| v.as_u64()),
            Some(25565));
    }

    #[test]
    fn settings_dirty_snapshot_changes_on_edit() {
        let form = wizard_base();
        let snap1 = form.snapshot();
        let mut edited = form.clone();
        edited.name = "Renamed".to_string();
        assert_ne!(snap1, edited.snapshot());
        assert_eq!(form.validate(&[]), Ok(()));
        assert_eq!(edited.validate(&[]), Ok(()));
    }
}
