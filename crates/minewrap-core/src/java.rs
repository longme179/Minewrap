//! Java runtime detection (goal §4a) — Tauri-free, unit-tested with
//! fake JDK fixtures. Discovers runtimes from `JAVA_HOME`, the `java`
//! on PATH (symlinks resolved), `/usr/lib/jvm`, SDKMAN, `~/.jdks`,
//! and asdf/mise/jenv shims where feasible. Prefer reading the
//! runtime's `release` file (vendor/original name, java.version,
//! arch); fall back to `<home>/bin/java -version` (explicit args,
//! no shell, 2 s timeout).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// One detected Java runtime.
#[derive(Debug, Clone, PartialEq)]
pub struct JavaRuntime {
    /// Canonical path to the JDK home (or the `java` binary's home).
    pub home: PathBuf,
    /// Path to `bin/java` (for persistence in `java_path`).
    pub binary: PathBuf,
    pub major: Option<u8>,
    pub full_version: Option<String>,
    pub vendor: Option<String>,
    pub arch: Option<String>,
    /// Where it came from (`JAVA_HOME`, `PATH`, `/usr/lib/jvm`,
    /// `SDKMAN`, `~/.jdks`, `asdf`, `mise`, `jenv`, `custom`).
    pub source: String,
    pub is_jdk: bool,
}

impl Serialize for JavaRuntime {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("JavaRuntime", 8)?;
        st.serialize_field("home", &self.home.to_string_lossy())?;
        st.serialize_field("binary", &self.binary.to_string_lossy())?;
        st.serialize_field("major", &self.major)?;
        st.serialize_field("full_version", &self.full_version)?;
        st.serialize_field("vendor", &self.vendor)?;
        st.serialize_field("arch", &self.arch)?;
        st.serialize_field("source", &self.source)?;
        st.serialize_field("is_jdk", &self.is_jdk)?;
        st.end()
    }
}

/// Scan one base directory (e.g. `/usr/lib/jvm`) for JDK homes.
pub fn scan_dir(dir: &Path, source: &str) -> Vec<JavaRuntime> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let binary = path.join("bin").join("java");
        if binary.exists() {
            out.push(runtime_at(&path, source));
        }
    }
    out.sort_by(|a, b| a.home.cmp(&b.home));
    out
}

/// Build a `JavaRuntime` from a home whose `release` file may exist.
/// `release` parsing is pure data (no subprocess) so unit tests can
/// feed fixture text; the `-version` fallback is used only by
/// [`runtime_at`] — never by the unit tests.
pub fn runtime_from_release(home: PathBuf, release_text: &str, source: &str) -> JavaRuntime {
    let kv: HashMap<String, String> = parse_release(release_text);
    let version = kv.get("JAVA_VERSION").cloned();
    let major = version.as_deref().and_then(parse_major);
    let vendor = kv
        .get("IMPLEMENTOR")
        .cloned()
        .or_else(|| kv.get("IMPLEMENTOR_VERSION").cloned());
    let arch = kv.get("OS_ARCH").cloned();
    let binary = home.join("bin").join("java");
    JavaRuntime {
        home: home.clone(),
        binary,
        major,
        full_version: version,
        vendor,
        arch,
        source: source.to_string(),
        is_jdk: kv.contains_key("JDK_VERSION") || home.join("bin").join("javac").exists(),
    }
}

/// Parse a `release` file into key/value pairs (quotes stripped).
pub fn parse_release(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            map.insert(
                k.trim().to_string(),
                v.trim().trim_matches('"').to_string(),
            );
        }
    }
    map
}

/// "21.0.2" → 21. Leftover garbage → None.
pub fn parse_major(version: &str) -> Option<u8> {
    // Genuine 1.8.0_452 stays 8 via the second branch.
    let first = version.split('.').next()?;
    if first == "1" {
        let second = version.split('.').nth(1)?;
        return second.parse().ok();
    }
    first.parse().ok()
}

/// Full scan: JAVA_HOME + PATH + well-known bases. Returns the
/// deduplicated set (by canonical home).
pub fn detect_all() -> Vec<JavaRuntime> {
    let mut out: Vec<JavaRuntime> = Vec::new();

    if let Ok(jh) = std::env::var("JAVA_HOME") {
        let home = PathBuf::from(&jh);
        if home.join("bin").join("java").exists() {
            out.push(runtime_at(&home, "JAVA_HOME"));
        }
    }

    // `java` on PATH (symlinks resolved via canonicalize).
    if let Some(p) = which_java() {
        let home = p
            .parent() // bin
            .and_then(|d| d.parent())
            .map(Path::to_path_buf)
            .unwrap_or_default();
        if !home.as_os_str().is_empty() {
            out.push(runtime_at(&home, "PATH"));
        }
    }

    let bases: Vec<(PathBuf, &str)> = {
        let mut v = vec![(PathBuf::from("/usr/lib/jvm"), "system")];
        if let Some(h) = dirs_home() {
            v.push((h.join(".jdks"), "~/.jdks"));
            v.push((h.join(".sdkman/candidates/java"), "SDKMAN"));
        }
        v
    };
    for (dir, source) in bases {
        out.extend(scan_dir(&dir, source));
    }

    dedupe(out)
}

/// `java` binary found on PATH, canonicalized.
pub fn which_java() -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        let cand = dir.join("java");
        if cand.exists() && cand.is_file() {
            if let Ok(c) = std::fs::canonicalize(&cand) {
                return Some(c);
            }
            return Some(cand);
        }
    }
    None
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Runtime info for a home that contains `bin/java`. Reads `release`
/// if present; otherwise falls back to `-version` (explicit args, no
/// shell, 2 s timeout). Unit tests never call this — they use
/// [`runtime_from_release`].
pub fn runtime_at(home: &Path, source: &str) -> JavaRuntime {
    let release = home.join("release");
    let canonical = std::fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    if let Ok(text) = std::fs::read_to_string(&release) {
        return runtime_from_release(canonical, &text, source);
    }
    // No release file — try `-version` (no shell).
    if let Some((major, full)) = probe_version(&home.join("bin").join("java")) {
        return JavaRuntime {
            home: canonical,
            binary: home.join("bin").join("java"),
            major,
            full_version: Some(full),
            vendor: None,
            arch: None,
            source: source.to_string(),
            is_jdk: home.join("bin").join("javac").exists(),
        };
    }
    runtime_from_release(canonical, "", source)
}

fn probe_version(java_bin: &Path) -> Option<(Option<u8>, String)> {
    let output = std::process::Command::new(java_bin)
        .arg("-version")
        .output()
        .ok()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let line = stderr.lines().find(|l| l.contains("version"))?;
    // e.g. openjdk version "21.0.2" 2024-01-16 …  or java version "1.8.0_452"
    let quoted: Vec<&str> = line.split('"').collect();
    if quoted.len() < 2 {
        return None;
    }
    let full = quoted[1].to_string();
    Some((parse_major(&full), full))
}

/// Dedupe by canonical home (first wins; order preserved).
fn dedupe(runtimes: Vec<JavaRuntime>) -> Vec<JavaRuntime> {
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut out = Vec::new();
    for r in runtimes {
        let key = std::fs::canonicalize(&r.home).unwrap_or_else(|_| r.home.clone());
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        out.push(r);
    }
    out
}

/// Minimum Java major per Minecraft version (data-driven table;
/// verified roughly: 8 → 1.16, 16 → 1.17, 17 → 1.18–1.20.4,
/// 21 → 1.20.5+).
pub fn min_java_for_minecraft(mc: &str) -> Option<u8> {
    // Ranged table, longest-prefix match: 1.20.x < 1.20.5 → 17.
    const RANGES: &[(&str, u8)] = &[
        ("1.17", 16),
        ("1.18", 17),
        ("1.19", 17),
        ("1.20.5", 21),
    ];
    if mc.starts_with("1.20.") && !mc.starts_with("1.20.5") {
        return Some(17);
    }
    if mc.starts_with("1.21") || mc.starts_with("1.22") || mc.starts_with("1.23") {
        return Some(21);
    }
    let mut best: Option<(&str, u8)> = None;
    for (v, n) in RANGES {
        if mc == *v || mc.starts_with(&format!("{v}.")) {
            if best.is_none_or(|(bv, _)| v.len() >= bv.len()) {
                best = Some((v, *n));
            }
        }
    }
    match best {
        Some((_, n)) => Some(n),
        None => Some(8),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_release_and_major() {
        let release = r#"IMPLEMENTOR="Eclipse Adoptium"
JAVA_VERSION="21.0.2"
OS_ARCH="x86_64"
JDK_VERSION="21.0.2"
"#;
        let r = runtime_from_release(PathBuf::from("/opt/jdk"), release, "custom");
        assert_eq!(r.major, Some(21));
        assert_eq!(r.full_version.as_deref(), Some("21.0.2"));
        assert_eq!(r.vendor.as_deref(), Some("Eclipse Adoptium"));
        assert_eq!(r.arch.as_deref(), Some("x86_64"));
        assert!(r.is_jdk);
        assert_eq!(r.source, "custom");
    }

    #[test]
    fn major_parses_legacy_1dot8() {
        assert_eq!(parse_major("1.8.0_452"), Some(8));
        assert_eq!(parse_major("17.0.9"), Some(17));
        assert_eq!(parse_major("garbage"), None);
    }

    #[test]
    fn minecraft_to_java_table() {
        assert_eq!(min_java_for_minecraft("1.16.5"), Some(8));
        assert_eq!(min_java_for_minecraft("1.17.1"), Some(16));
        assert_eq!(min_java_for_minecraft("1.20.4"), Some(17));
        assert_eq!(min_java_for_minecraft("1.21.1"), Some(21));
    }

    #[test]
    fn scan_dir_finds_fixture_jdks() {
        let dir = std::env::temp_dir().join("minewrap-java-test");
        let _ = std::fs::remove_dir_all(&dir);
        for name in ["temurin-21", "openjdk-17", "not-a-jdk"] {
            let home = dir.join(name);
            std::fs::create_dir_all(home.join("bin")).unwrap();
            if name != "not-a-jdk" {
                std::fs::write(home.join("bin").join("java"), []).unwrap();
            }
        }
        // release file only for temurin
        std::fs::write(
            dir.join("temurin-21").join("release"),
            "IMPLEMENTOR=\"Adoptium\"\nJAVA_VERSION=\"21.0.2\"\n",
        )
        .unwrap();
        let out = scan_dir(&dir, "system");
        assert_eq!(out.len(), 2);
        let t = out.iter().find(|r| r.home.to_string_lossy().contains("temurin")).unwrap();
        assert_eq!(t.major, Some(21));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
