//! Launch detection (goal §3) — scans a work dir for server jars,
//! run scripts, args files, and reads `version.json` inside the jar
//! (vanilla/Forge/NeoForge/Paper/Spigot/etc.) to report the detected
//! type + Minecraft version. Jar inspection uses only the zip
//! central directory (no shell, no unzip binary).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::java::min_java_for_minecraft;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LaunchCandidate {
    pub path: PathBuf,
    pub kind: LaunchKind,
    pub server_type: Option<String>,
    pub mc_version: Option<String>,
    pub suggested_launch_args: Vec<String>,
    pub min_java: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchKind {
    Jar,
    ArgsFile,
    Script,
    Custom,
}

impl LaunchCandidate {
    pub fn display(&self) -> String {
        match self.kind {
            LaunchKind::Jar => format!("Jar — {}", self.path.to_string_lossy()),
            LaunchKind::ArgsFile => format!("Args file — {}", self.path.to_string_lossy()),
            LaunchKind::Script => format!("Script — {}", self.path.to_string_lossy()),
            LaunchKind::Custom => format!("Custom — {}", self.path.to_string_lossy()),
        }
    }
}

/// Scan a work dir for launch candidates (confinement: the caller has
/// already canonicalized the work dir; we stay inside it).
pub fn scan_work_dir(work_dir: &Path) -> Vec<LaunchCandidate> {
    let mut out: Vec<LaunchCandidate> = Vec::new();
    let entries = match std::fs::read_dir(work_dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if !path.is_file() {
            continue;
        }
        if name.ends_with(".jar") {
            let (ty, ver) = inspect_jar(&path);
            let min_java = ver.as_deref().and_then(min_java_for_minecraft);
            out.push(LaunchCandidate {
                suggested_launch_args: vec!["-jar".into(), name.clone(), "nogui".into()],
                path,
                kind: LaunchKind::Jar,
                server_type: ty,
                mc_version: ver.clone(),
                min_java,
            });
        } else if name == "run.sh" || name.ends_with(".sh") {
            out.push(LaunchCandidate {
                path,
                kind: LaunchKind::Script,
                server_type: None,
                mc_version: None,
                suggested_launch_args: vec!["./".to_string() + &name, "nogui".to_string()],
                min_java: None,
            });
        } else if name == "user_jvm_args.txt" || name == "unix_args.txt" {
            out.push(LaunchCandidate {
                path,
                kind: LaunchKind::ArgsFile,
                server_type: None,
                mc_version: None,
                suggested_launch_args: vec![format!("@{name}"), "nogui".into()],
                min_java: None,
            });
        }
    }
    // Forge/NeoForge `libraries/**/unix_args.txt`
    let libs = work_dir.join("libraries");
    if libs.exists() {
        for found in walk_files(&libs, 3) {
            if found
                .file_name()
                .map(|s| s == "unix_args.txt")
                .unwrap_or(false)
            {
                if let Ok(rel) = found.strip_prefix(work_dir) {
                    out.push(LaunchCandidate {
                        suggested_launch_args: vec![
                            "@user_jvm_args.txt".into(),
                            format!("@{}", rel.to_string_lossy()),
                            "nogui".into(),
                        ],
                        path: found,
                        kind: LaunchKind::ArgsFile,
                        server_type: None,
                        mc_version: None,
                        min_java: None,
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Read `version.json` inside a jar via the zip central directory
/// (pure `std::fs`, no shell, no external `unzip`).
pub fn inspect_jar(jar: &Path) -> (Option<String>, Option<String>) {
    let data = match std::fs::read(jar) {
        Ok(d) => d,
        Err(_) => return (None, None),
    };
    let Some(ver_json) = find_zip_entry(&data, "version.json") else {
        return (None, None);
    };
    let text = match std::str::from_utf8(&ver_json) {
        Ok(t) => t,
        Err(_) => return (None, None),
    };
    // `"id": "1.21.1-…"` (vanilla puts `"id": "1.21.1"`; Forge
    // `server.jar` embeds `"id": "1.21.1-…-server"` or
    // `user_info` data). Best-effort extraction; the UI shows
    // "Detected: <type> <version>" with an override.
    let id = extract_json_string(text, "id").unwrap_or_default();
    let name = extract_json_string(text, "name").unwrap_or_default();
    let type_guess = if name.to_lowercase().contains("forge") {
        Some("Forge".to_string())
    } else if name.to_lowercase().contains("neoforge") {
        Some("NeoForge".to_string())
    } else if name.to_lowercase().contains("paper") {
        Some("Paper".to_string())
    } else if name.to_lowercase().contains("spigot") {
        Some("Spigot".to_string())
    } else if id.is_empty() {
        None
    } else {
        Some("Vanilla".to_string())
    };
    (type_guess, (!id.is_empty()).then_some(id))
}

fn extract_json_string(text: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\"");
    let start = text.find(&pat)?;
    let rest = &text[start + pat.len()..];
    let colon = rest.find(':')?;
    let after = rest[colon + 1..].trim_start();
    if !after.starts_with('"') {
        return None;
    }
    let end = after[1..].find('"')?;
    Some(after[1..end].to_string())
}

/// Minimal zip reader: walk the central directory for `path`.
fn find_zip_entry(zip: &[u8], wanted: &str) -> Option<Vec<u8>> {
    if zip.len() < 22 || !zip.windows(4).any(|w| w == b"PK\x05\x06") {
        return None;
    }
    // EOCD is at the end; take the last occurrence.
    let eocd = zip.windows(4).rposition(|w| w == b"PK\x05\x06")?;
    if eocd + 22 > zip.len() {
        return None;
    }
    let u32 = |i: usize| {
        u32::from_le_bytes([
            zip[eocd + i],
            zip[eocd + i + 1],
            zip[eocd + i + 2],
            zip[eocd + i + 3],
        ]) as usize
    };
    let cd_offset = u32(16);
    let mut p = cd_offset;
    while p + 46 <= zip.len() && &zip[p..p + 4] == b"PK\x01\x02" {
        let method = u16::from_le_bytes([zip[p + 10], zip[p + 11]]);
        let compressed_size =
            u32::from_le_bytes([zip[p + 20], zip[p + 21], zip[p + 22], zip[p + 23]]) as usize;
        let name_len = u16::from_le_bytes([zip[p + 28], zip[p + 29]]) as usize;
        let extra_len = u16::from_le_bytes([zip[p + 30], zip[p + 31]]) as usize;
        let comment_len = u16::from_le_bytes([zip[p + 32], zip[p + 33]]) as usize;
        let header = p + 46;
        let name = std::str::from_utf8(&zip[header..header + name_len]).ok()?;
        if name == wanted && method == 0 {
            // Stored (method 0) — local header at the offset.
            let lo =
                u32::from_le_bytes([zip[p + 42], zip[p + 43], zip[p + 44], zip[p + 45]]) as usize;
            if lo + 30 <= zip.len() && &zip[lo..lo + 4] == b"PK\x03\x04" {
                let lname = u16::from_le_bytes([zip[lo + 26], zip[lo + 27]]) as usize;
                let lextra = u16::from_le_bytes([zip[lo + 28], zip[lo + 29]]) as usize;
                let start = lo + 30 + lname + lextra;
                let end = (start + compressed_size).min(zip.len());
                return Some(zip[start..end].to_vec());
            }
            return None;
        }
        p = header + name_len + extra_len + comment_len;
    }
    None
}

fn walk_files(dir: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if depth == 0 {
        return out;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk_files(&path, depth - 1));
        } else if path.is_file() {
            out.push(path);
        }
    }
    out
}

/// Three launch modes (goal §3): Jar (`-jar x.jar nogui`),
/// Args-file (`@user_jvm_args.txt @libraries/.../unix_args.txt nogui`),
/// Custom (free-form, with a confinement check against the work dir).
pub fn suggested_for_mode(mode: &str, work_dir: &Path, jar_name: &str) -> Option<Vec<String>> {
    match mode {
        "jar" => Some(vec!["-jar".into(), jar_name.into(), "nogui".into()]),
        "args-file" => {
            let args_file = work_dir.join("user_jvm_args.txt");
            if !args_file.exists() {
                let libs = work_dir.join("libraries");
                let mut found = None;
                for f in walk_files(&libs, 3) {
                    if f.file_name().map(|s| s == "unix_args.txt").unwrap_or(false) {
                        found = Some(f);
                        break;
                    }
                }
                let f = found?;
                let rel = f.strip_prefix(work_dir).ok()?;
                return Some(vec![
                    "@user_jvm_args.txt".into(),
                    format!("@{}", rel.to_string_lossy()),
                    "nogui".into(),
                ]);
            }
            Some(vec!["@user_jvm_args.txt".into(), "nogui".into()])
        }
        "custom" => None,
        _ => None,
    }
}

/// Report what a chosen work dir contains (live validation UX).
#[derive(Debug, Clone, Serialize)]
pub struct WorkDirReport {
    pub exists: bool,
    pub readable: bool,
    pub writable: bool,
    pub has_server_properties: bool,
    pub has_eula: bool,
    pub has_world: bool,
    pub has_mods: bool,
    pub jar_candidates: Vec<String>,
    pub notes: Vec<String>,
}

impl Default for WorkDirReport {
    fn default() -> Self {
        Self {
            exists: false,
            readable: false,
            writable: false,
            has_server_properties: false,
            has_eula: false,
            has_world: false,
            has_mods: false,
            jar_candidates: Vec::new(),
            notes: Vec::new(),
        }
    }
}

pub fn report_work_dir(dir: &Path) -> WorkDirReport {
    let exists = dir.exists();
    if !exists {
        return WorkDirReport {
            notes: vec![format!("{} does not exist", dir.to_string_lossy())],
            ..WorkDirReport::default()
        };
    }
    let mut r = WorkDirReport::default();
    r.exists = true;
    r.readable = std::fs::read_dir(dir).is_ok();
    r.writable = {
        let probe = dir.join(".minewrap_write_test");
        std::fs::write(&probe, b"").is_ok()
    };
    if r.writable {
        let _ = std::fs::remove_file(dir.join(".minewrap_write_test"));
    }
    r.has_server_properties = dir.join("server.properties").exists();
    r.has_eula = dir.join("eula.txt").exists();
    r.has_world = dir.join("world").exists();
    r.has_mods = dir.join("mods").exists();
    let seen: HashSet<String> = scan_work_dir(dir)
        .into_iter()
        .map(|c| c.path.to_string_lossy().to_string())
        .collect();
    r.jar_candidates = seen.into_iter().collect();
    if !r.has_server_properties {
        r.notes
            .push("No server.properties yet (created on first run)".into());
    }
    if !r.has_eula {
        r.notes.push("No eula.txt yet".into());
    }
    if !r.writable {
        r.notes.push("Work dir is not writable".into());
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_finds_jars_and_args_files() {
        let dir = std::env::temp_dir().join("minewrap-launch-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("libraries/net/minecraft")).unwrap();
        std::fs::write(dir.join("server.jar"), b"PK").unwrap();
        std::fs::write(dir.join("run.sh"), b"#!/bin/sh\n").unwrap();
        std::fs::write(dir.join("user_jvm_args.txt"), b"-Xmx2G\n").unwrap();
        std::fs::write(
            dir.join("libraries/net/minecraft/unix_args.txt"),
            b"@args\n",
        )
        .unwrap();
        let out = scan_work_dir(&dir);
        assert!(out.iter().any(|c| c.kind == LaunchKind::Jar));
        assert!(out.iter().any(|c| c.kind == LaunchKind::Script));
        assert!(out.iter().any(|c| c.kind == LaunchKind::ArgsFile));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn report_work_dir_flags() {
        let dir = std::env::temp_dir().join("minewrap-launch-test2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("server.properties"), b"server-port=25565\n").unwrap();
        std::fs::create_dir_all(dir.join("world")).unwrap();
        let r = report_work_dir(&dir);
        assert!(r.exists && r.readable && r.writable);
        assert!(r.has_server_properties && r.has_world);
        assert!(!r.has_eula);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
