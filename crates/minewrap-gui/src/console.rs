//! Pure console logic for the Minewrap Slint GUI (no UI dependencies).
//!
//! Ported from the C1 spike: a 1000-line FIFO ring, per-level tags and a
//! monotonic `seq` guard so REST `since_seq` backfill + WS replay never
//! duplicate a line.

use std::collections::VecDeque;

/// Exactly 1000 console lines (the spike's measured, stable ring size).
pub const RING: usize = 1000;

/// Level indices used by the Slint markup (`level-color`).
pub const LEVEL_INFO: i32 = 0;
pub const LEVEL_WARN: i32 = 1;
pub const LEVEL_ERROR: i32 = 2;
pub const LEVEL_DEBUG: i32 = 3;

/// One line ready for the UI model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleLine {
    pub seq: u64,
    pub ts: String,
    pub tag: String,
    pub level: i32,
    pub msg: String,
}

impl ConsoleLine {
    /// Clipboard/plain-text form (`ts [TAG] msg`).
    pub fn display(&self) -> String {
        format!("{} [{}] {}", self.ts, self.tag, self.msg)
    }
}

/// Outcome of inserting a line into the ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushOutcome {
    /// Accepted; `evicted` says the oldest UI row must be dropped too.
    Accepted { evicted: bool },
    /// Older than (or equal to) the highest seq already in the ring.
    Duplicate,
}

/// 1000-line FIFO with a monotonic sequence guard.
#[derive(Debug)]
pub struct Ring {
    lines: VecDeque<ConsoleLine>,
    last_seq: u64,
}

impl Default for Ring {
    fn default() -> Self {
        Self::new()
    }
}

impl Ring {
    pub fn new() -> Self {
        Self {
            lines: VecDeque::with_capacity(RING),
            last_seq: 0,
        }
    }

    /// Drop every line and re-arm backfill (`since_seq = 0`).
    pub fn reset(&mut self) {
        self.lines.clear();
        self.last_seq = 0;
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }

    pub fn first_seq(&self) -> u64 {
        self.lines.front().map(|l| l.seq).unwrap_or(0)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ConsoleLine> {
        self.lines.iter()
    }

    /// Insert one line; stale/duplicate seqs are ignored.
    pub fn push(&mut self, line: ConsoleLine) -> PushOutcome {
        if line.seq <= self.last_seq {
            return PushOutcome::Duplicate;
        }
        self.last_seq = line.seq;
        let evicted = self.lines.len() == RING;
        if evicted {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
        PushOutcome::Accepted { evicted }
    }
}

/// Parse the Minecraft log level from a raw line.
///
/// Handles both shapes seen in the wild: `[19:04:11] [INFO] …` and
/// `[Server thread/WARN]: …`; unknown lines default to INFO.
pub fn parse_level(line: &str) -> i32 {
    let upper = line.to_ascii_uppercase();
    if upper.contains("ERROR") || upper.contains("FATAL") {
        LEVEL_ERROR
    } else if upper.contains("WARN") {
        LEVEL_WARN
    } else if upper.contains("DEBUG") {
        LEVEL_DEBUG
    } else {
        LEVEL_INFO
    }
}

/// Strip a leading `[HH:MM:SS] ` daemon timestamp and `[TAG] `/`[thread/TAG]:`
/// prefix. Returns `(tag, message)`; an absent tag falls back to the
/// level name.
fn split_prefixes(raw: &str, level: i32) -> (String, String) {
    let mut rest = raw.trim_end_matches(['\r', '\n']);
    if let Some(after) = strip_bracket_prefix(rest) {
        rest = after;
    }
    let mut tag = level_name(level).to_string();
    if rest.starts_with('[') {
        if let Some(close) = rest.find(']') {
            let candidate = &rest[1..close];
            let tail = rest[close + 1..]
                .strip_prefix(':')
                .unwrap_or(&rest[close + 1..]);
            let tail = tail.strip_prefix(' ').unwrap_or(tail);
            // Vanilla shape: `[Server thread/INFO]: …` — take the level
            // after the last `/`; spike shape: `[WARN] …`.
            let name = candidate.rsplit('/').next().unwrap_or(candidate);
            let known = matches!(name, "INFO" | "WARN" | "ERROR" | "DEBUG" | "TRACE" | "FATAL");
            let plain = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_');
            if known || plain {
                tag = if known { name.to_string() } else { candidate.to_string() };
                rest = tail;
            }
        }
    }
    (tag, rest.to_string())
}

/// If `s` starts with `[...]` followed by whitespace, return the tail.
fn strip_bracket_prefix(s: &str) -> Option<&str> {
    if !s.starts_with('[') {
        return None;
    }
    let close = s.find(']')?;
    let tail = s[close + 1..].strip_prefix(' ')?;
    Some(tail)
}

fn level_name(level: i32) -> &'static str {
    match level {
        LEVEL_WARN => "WARN",
        LEVEL_ERROR => "ERROR",
        LEVEL_DEBUG => "DEBUG",
        _ => "INFO",
    }
}

/// Build a UI line from a daemon `LogLine` (`seq`, `ts`, raw `line`).
pub fn make_line(seq: u64, ts: String, raw: &str) -> ConsoleLine {
    let level = parse_level(raw);
    let (tag, msg) = split_prefixes(raw, level);
    ConsoleLine {
        seq,
        ts,
        tag,
        level,
        msg,
    }
}

/// `HH:MM:SS` from the daemon's RFC3339 timestamp.
pub fn format_ts(ts: &chrono::DateTime<chrono::Utc>) -> String {
    ts.format("%H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn line(seq: u64) -> ConsoleLine {
        make_line(seq, "12:00:00".into(), "[12:00:00] [INFO] hello")
    }

    #[test]
    fn level_parsing_handles_both_log_shapes() {
        assert_eq!(parse_level("[19:04:11] [INFO] tick"), LEVEL_INFO);
        assert_eq!(parse_level("[19:04:11] [WARN] slow tick"), LEVEL_WARN);
        assert_eq!(parse_level("[Server thread/ERROR]: boom"), LEVEL_ERROR);
        assert_eq!(parse_level("[19:04:11] [DEBUG] chunk"), LEVEL_DEBUG);
        assert_eq!(parse_level("plain line"), LEVEL_INFO);
    }

    #[test]
    fn line_splits_timestamp_and_tag() {
        let l = make_line(7, "19:04:11".into(), "[19:04:11] [WARN] tick took 41ms");
        assert_eq!(l.seq, 7);
        assert_eq!(l.tag, "WARN");
        assert_eq!(l.level, LEVEL_WARN);
        assert_eq!(l.msg, "tick took 41ms");
        assert_eq!(l.display(), "19:04:11 [WARN] tick took 41ms");
    }

    #[test]
    fn vanilla_thread_prefix_is_stripped() {
        let l = make_line(
            2,
            "10:00:00".into(),
            "[10:00:00] [Server thread/INFO]: Done (1.234s)!",
        );
        assert_eq!(l.tag, "INFO");
        assert_eq!(l.msg, "Done (1.234s)!");
    }

    #[test]
    fn line_without_tag_uses_level_name() {
        let l = make_line(1, "10:00:00".into(), "Done (1.234s)! For help");
        assert_eq!(l.tag, "INFO");
        assert_eq!(l.msg, "Done (1.234s)! For help");
    }

    #[test]
    fn ring_keeps_exactly_the_newest_1000() {
        let mut ring = Ring::new();
        for seq in 1..=2500 {
            ring.push(line(seq));
        }
        assert_eq!(ring.len(), RING);
        assert_eq!(ring.first_seq(), 1501);
        assert_eq!(ring.last_seq(), 2500);
        assert_eq!(ring.iter().next().unwrap().seq, 1501);
        assert_eq!(ring.iter().last().unwrap().seq, 2500);
    }

    #[test]
    fn ring_rejects_duplicates_and_out_of_order_lines() {
        let mut ring = Ring::new();
        assert!(matches!(
            ring.push(line(5)),
            PushOutcome::Accepted { evicted: false }
        ));
        assert_eq!(ring.push(line(5)), PushOutcome::Duplicate);
        assert_eq!(ring.push(line(4)), PushOutcome::Duplicate);
        assert_eq!(ring.len(), 1);
        assert_eq!(ring.last_seq(), 5);
    }

    #[test]
    fn ring_reports_eviction_once_full() {
        let mut ring = Ring::new();
        for seq in 1..=RING as u64 {
            assert!(matches!(
                ring.push(line(seq)),
                PushOutcome::Accepted { evicted: false }
            ));
        }
        assert!(matches!(
            ring.push(line(RING as u64 + 1)),
            PushOutcome::Accepted { evicted: true }
        ));
        assert_eq!(ring.len(), RING);
        assert_eq!(ring.first_seq(), 2);
    }

    #[test]
    fn reset_rearms_backfill() {
        let mut ring = Ring::new();
        ring.push(line(99));
        ring.reset();
        assert!(ring.is_empty());
        assert_eq!(ring.last_seq(), 0);
        assert!(matches!(
            ring.push(line(1)),
            PushOutcome::Accepted { evicted: false }
        ));
    }

    #[test]
    fn format_ts_is_hh_mm_ss() {
        let ts = chrono::Utc.with_ymd_and_hms(2026, 11, 11, 19, 4, 5).unwrap();
        assert_eq!(format_ts(&ts), "19:04:05");
    }
}
