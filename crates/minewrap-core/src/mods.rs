//! `mods` directory manager: list, search/filter, toggle `.jar <-> .jar.disabled`.
//!
//! Jar metadata (Forge/NeoForge `META-INF/*.toml`) is stretch (Phase 4);
//! toggling is pure rename with confinement to the instance `mods` dir.

use std::path::{Path, PathBuf};

use crate::properties::confine_path;

/// One entry in `mods/`.
#[derive(Debug, Clone)]
pub struct ModEntry {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub enabled: bool,
}

/// List the `mods` folder (create it if missing).
pub fn list_mods(instance_dir: &Path) -> Result<Vec<ModEntry>, String> {
    let mods_dir = instance_dir.join("mods");
    if !mods_dir.exists() {
        std::fs::create_dir_all(&mods_dir).map_err(|e| e.to_string())?;
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&mods_dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        // Only jars (enabled `.jar` and disabled `.jar.disabled`) are mods.
        if !name.ends_with(".jar") && !name.ends_with(".jar.disabled") {
            continue;
        }
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let enabled = !name.ends_with(".jar.disabled");
        entries.push(ModEntry {
            name,
            size,
            enabled,
            path,
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// Toggle a mod by renaming inside `mods/` (confinement-checked).
pub fn toggle_mod(instance_dir: &Path, name: &str) -> Result<bool, String> {
    // Reject anything with a path separator (confinement, paranoia level).
    if name.contains('/') || name.contains('\\') {
        return Err("mod name must not contain a path separator".into());
    }
    let mods_dir = instance_dir.join("mods");
    let current = mods_dir.join(name);
    let canonical_dir = std::fs::canonicalize(&mods_dir).map_err(|e| e.to_string())?;
    if !current.exists() {
        if let Ok(canonical) = confine_path(instance_dir, &format!("mods/{name}")) {
            if canonical == current || canonical.starts_with(&canonical_dir) {
                return Err(format!("mod {name} not found"));
            }
        }
        return Err(format!("mod {name} not found"));
    }
    let to = if name.ends_with(".jar.disabled") {
        let base = name.trim_end_matches(".jar.disabled");
        mods_dir.join(format!("{base}.jar"))
    } else if name.ends_with(".jar") {
        mods_dir.join(format!("{name}.disabled"))
    } else {
        return Err(format!("not a mod jar: {name}"));
    };
    std::fs::rename(&current, &to).map_err(|e| e.to_string())?;
    Ok(to.file_name().map(|n| !n.to_string_lossy().ends_with(".jar.disabled")).unwrap_or(false))
}

/// Human-readable size (kB/MB).
pub fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{:.1} {}", size, UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_renames_jar() {
        let dir = std::env::temp_dir().join("minewrap-mods-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("mods")).unwrap();
        std::fs::write(dir.join("mods/test.jar"), b"jar").unwrap();

        let enabled = toggle_mod(&dir, "test.jar").unwrap();
        assert!(!enabled);
        assert!(!dir.join("mods/test.jar").exists());
        assert!(dir.join("mods/test.jar.disabled").exists());

        let enabled = toggle_mod(&dir, "test.jar.disabled").unwrap();
        assert!(enabled);
        assert!(dir.join("mods/test.jar").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_and_pathy_names_fail() {
        let dir = std::env::temp_dir().join("minewrap-mods-test2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("mods")).unwrap();
        assert!(toggle_mod(&dir, "nope.jar").is_err());
        assert!(toggle_mod(&dir, "../evil.jar").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
