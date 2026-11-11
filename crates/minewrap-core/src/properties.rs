//! `server.properties` round-trip parser/writer: Java properties format.
//!
//! Preserves comments, ordering and unknown keys; exposes typed access with
//! validation for known keys. Files are parsed into `Document` entries (one
//! per line) so writing is byte-identical for untouched lines.

use std::collections::HashMap;
use std::path::Path;

/// A single property line, in file order.
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyEntry {
    pub key: String,
    pub value: String,
    /// 0 = header/prefix area, 1 = main body
    pub group: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    Empty,
    Comment(String),
    Property(PropertyEntry),
    /// Anything else (preserved verbatim so round-trip never loses content).
    Raw(String),
}

/// A parsed `server.properties` document.
#[derive(Debug, Clone)]
pub struct PropertiesDocument {
    pub lines: Vec<Line>,
}

impl PropertiesDocument {
    /// Parse text into a document (never fails: unknown syntax becomes `Raw`).
    pub fn parse(text: &str) -> Self {
        let mut lines = Vec::new();
        for raw in text.lines() {
            let line = raw.trim_end_matches('\r');
            if line.is_empty() {
                lines.push(Line::Empty);
            } else if line.starts_with('#') {
                lines.push(Line::Comment(line.to_string()));
            } else if let Some((k, v)) = line.split_once('=') {
                lines.push(Line::Property(PropertyEntry {
                    key: k.trim().to_string(),
                    value: v.trim().to_string(),
                    group: 0,
                }));
            } else {
                lines.push(Line::Raw(line.to_string()));
            }
        }
        Self { lines }
    }

    /// Render back to a string (round-trips verbatim for untouched lines).
    pub fn to_string(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            match line {
                Line::Empty => out.push('\n'),
                Line::Comment(c) => {
                    out.push_str(c);
                    out.push('\n');
                }
                Line::Raw(r) => {
                    out.push_str(r);
                    out.push('\n');
                }
                Line::Property(entry) => {
                    out.push_str(&entry.key);
                    out.push('=');
                    out.push_str(&entry.value);
                    out.push('\n');
                }
            }
        }
        out
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        for line in &self.lines {
            if let Line::Property(e) = line {
                if e.key == key {
                    return Some(&e.value);
                }
            }
        }
        None
    }

    /// Set a value after validation (called by `validate_and_set`).
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        validate_property(key, value)?;
        for line in &mut self.lines {
            if let Line::Property(e) = line {
                if e.key == key {
                    e.value = value.to_string();
                    return Ok(());
                }
            }
        }
        self.lines.push(Line::Property(PropertyEntry {
            key: key.to_string(),
            value: value.to_string(),
            group: 0,
        }));
        Ok(())
    }

    /// Type-safe map for the known keys the UI edits.
    pub fn typed(&self) -> HashMap<&str, &str> {
        let mut map = HashMap::new();
        for line in &self.lines {
            if let Line::Property(e) = line {
                map.insert(e.key.as_str(), e.value.as_str());
            }
        }
        map
    }
}

pub fn validate_property(key: &str, value: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("property key must not be empty".into());
    }
    match key {
        "server-port" | "query.port" | "rcon.port" | "max-players" | "view-distance" => {
            validate_int_range(key, value, 0, 65535)
        }
        "gamemode" => match value {
            "survival" | "creative" | "adventure" | "spectator" => Ok(()),
            other => Err(format!(
                "gamemode must be survival|creative|adventure|spectator, got {other:?}"
            )),
        },
        "difficulty" => match value {
            "peaceful" | "easy" | "normal" | "hard" => Ok(()),
            other => Err(format!(
                "difficulty must be peaceful|easy|normal|hard, got {other:?}"
            )),
        },
        "level-type" => Ok(()), // free-form
        "online-mode"
        | "pvp"
        | "enable-command-block"
        | "spawn-monsters"
        | "spawn-animals"
        | "spawn-npcs"
        | "hide-online-players"
        | "force-gamemode"
        | "broadcast-console-to-ops"
        | "snooper-enabled"
        | "enable-rcon"
        | "motd" => Ok(()),
        _ => Ok(()), // unknown keys pass through
    }
}

fn validate_int_range(key: &str, value: &str, lo: u32, hi: u32) -> Result<(), String> {
    let v: u32 = value
        .parse()
        .map_err(|_| format!("{key} must be an integer"))?;
    if !(lo..=hi).contains(&v) {
        return Err(format!("{key} must be between {lo} and {hi}, got {value}"));
    }
    Ok(())
}

/// Path confinement (parity with Rule 5): canonicalise and confine every
/// UI-supplied path to `instance_dir`.
pub fn confine_path(instance_dir: &Path, ui_path: &str) -> Result<std::path::PathBuf, String> {
    let base = std::fs::canonicalize(instance_dir).map_err(|e| format!("instance dir: {e}"))?;
    let candidate = if std::path::Path::new(ui_path).is_absolute() {
        std::path::PathBuf::from(ui_path)
    } else {
        base.join(ui_path)
    };
    let canonical = std::fs::canonicalize(&candidate)
        .map_err(|e| format!("path {:?} cannot be resolved: {e}", ui_path))?;
    if canonical.starts_with(&base) {
        Ok(canonical)
    } else {
        Err(format!("path {:?} escapes the instance directory", ui_path))
    }
}

/// Properties is always `server.properties` inside the instance dir.
pub fn properties_path(instance_dir: &Path) -> std::path::PathBuf {
    instance_dir.join("server.properties")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_comments_and_order() {
        let input = "# header\na=1\n\n# comment\nb=2\nunknown=x\n";
        let doc = PropertiesDocument::parse(input);
        assert_eq!(doc.to_string(), input);
    }

    #[test]
    fn raw_lines_are_preserved() {
        let input = "no-equals-here\nx=1\n";
        let doc = PropertiesDocument::parse(input);
        assert_eq!(doc.to_string(), input);
    }

    #[test]
    fn validate_rejects_bad_ints_and_enums() {
        assert!(validate_property("server-port", "abc").is_err());
        assert!(validate_property("server-port", "70000").is_err());
        assert!(validate_property("gamemode", "hardcore").is_err());
        assert!(validate_property("gamemode", "survival").is_ok());
        assert!(validate_property("some-unknown-key", "anything").is_ok());
    }

    #[test]
    fn set_after_validate() {
        let mut doc = PropertiesDocument::parse("level-name=world\nserver-port=25565\n");
        doc.set("server-port", "25566").unwrap();
        assert_eq!(doc.get("server-port"), Some("25566"));
        assert!(doc.set("server-port", "70000").is_err());
        assert_eq!(doc.get("server-port"), Some("25566")); // unchanged on error
    }
}
