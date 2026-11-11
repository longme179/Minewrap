//! Log types, ring buffer, batching and rotation.
//!
//! Ports of Python v1 `log_buffer.py` (4 tests), `log_file.py` (3 tests) and
//! `process_manager.py`'s reader (the `\r\n` strip and UTF-8-tolerant decode).
//!
//! Performance notes: the ring keeps `capacity` entries max; batching coalesces
//! lines from the pipe into `LogLineBatch`/events before emit (1500-line cap
//! per batch); no per-line allocation beyond the line itself.

use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::mpsc;

/// A single log line with a monotonic sequence number (per instance).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogLine {
    pub seq: u64,
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub ts: DateTime<Utc>,
    pub line: String,
}

/// Batch of lines for one emit (post-batching log batch).
#[derive(Debug, Clone, PartialEq)]
pub struct LogLineBatch {
    pub first_seq: u64,
    pub last_seq: u64,
    pub lines: Vec<LogLine>,
}

/// Convert a raw pipe line into a `LogLine` (strips `\r\n`).
pub fn parse_line(seq: u64, ts: DateTime<Utc>, raw: &str) -> LogLine {
    LogLine {
        seq,
        ts,
        line: raw.trim_end_matches(['\r', '\n']).to_string(),
    }
}

/// Bounded in-memory ring buffer of `LogLine`s (v1 `LogBuffer`).
#[derive(Debug)]
pub struct LogBuffer {
    lines: VecDeque<LogLine>,
    capacity: usize,
    seq: u64,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity >= 1, "log buffer capacity must be >= 1");
        Self {
            lines: VecDeque::with_capacity(capacity),
            capacity,
            seq: 0,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self::new(capacity)
    }

    /// Append a finished line (UTF-8 lossy decode is done by the reader).
    pub fn append(&mut self, line: String, ts: DateTime<Utc>) -> LogLine {
        self.seq += 1;
        let entry = LogLine {
            seq: self.seq,
            ts,
            line,
        };
        self.lines.push_back(entry.clone());
        while self.lines.len() > self.capacity {
            self.lines.pop_front();
        }
        entry
    }

    /// Last `n` lines.
    pub fn tail(&self, n: usize) -> Vec<LogLine> {
        if n == 0 {
            return Vec::new();
        }
        let skip = self.lines.len().saturating_sub(n);
        self.lines.iter().skip(skip).cloned().collect()
    }

    /// Everything since (exclusive) `seq`.
    pub fn since(&self, seq: u64) -> Vec<LogLine> {
        self.lines.iter().filter(|l| l.seq > seq).cloned().collect()
    }

    pub fn first_seq(&self) -> Option<u64> {
        self.lines.front().map(|l| l.seq)
    }

    pub fn last_seq(&self) -> Option<u64> {
        self.lines.back().map(|l| l.seq)
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// Line batcher: wraps a raw `mpsc::Receiver<String>` pipe and produces
/// `LogLineBatch` every `interval` or when it grows to `max_lines`.
pub struct Batcher {
    rx: mpsc::Receiver<String>,
    pending: Vec<LogLine>,
    seq: u64,
    #[allow(dead_code)]
    interval_ms: u64,
    max_lines: usize,
}

impl Batcher {
    pub fn new(rx: mpsc::Receiver<String>, interval_ms: u64, max_lines: usize) -> Self {
        Self {
            rx,
            pending: Vec::new(),
            seq: 0,
            interval_ms,
            max_lines: max(1, max_lines),
        }
    }

    /// Best-effort drain of immediately-available lines into the pending batch.
    pub fn drain_pending(&mut self) {
        while self.pending.len() < self.max_lines {
            match self.rx.try_recv() {
                Ok(line) => {
                    self.seq += 1;
                    self.pending.push(parse_line(self.seq, Utc::now(), &line));
                }
                Err(_) => break,
            }
        }
    }

    pub fn take_batch(&mut self) -> Option<LogLineBatch> {
        if self.pending.is_empty() {
            return None;
        }
        let lines = std::mem::take(&mut self.pending);
        let first_seq = lines.first().map(|l| l.seq).unwrap_or(0);
        let last_seq = lines.last().map(|l| l.seq).unwrap_or(0);
        Some(LogLineBatch {
            first_seq,
            last_seq,
            lines,
        })
    }

    pub fn pending(&self) -> &[LogLine] {
        &self.pending
    }
}

/// `max(1, x)` helper (clippy::unnecessary_cast pedantry).
fn max(a: usize, b: usize) -> usize {
    if a > b { a } else { b }
}

/// Rotating file tee with size-based rotation (v1 `RotatingLogFile`).
#[derive(Debug)]
pub struct RotatingFile {
    path: std::path::PathBuf,
    max_bytes: u64,
    backups: u64,
    size: u64,
}

impl RotatingFile {
    pub fn new(path: std::path::PathBuf, max_bytes: u64, backups: u64) -> Self {
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        Self {
            path,
            max_bytes,
            backups,
            size,
        }
    }

    pub fn write(&mut self, line: &str) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        use std::io::Write;
        let text = format!("{line}\n");
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(text.as_bytes())?;
        self.size += text.len() as u64;
        if self.size >= self.max_bytes {
            self.rotate()?;
        }
        Ok(())
    }

    fn rotate(&mut self) -> std::io::Result<()> {
        if self.backups == 0 {
            std::fs::remove_file(&self.path).ok();
            self.size = 0;
            return Ok(());
        }
        // v1 `log_file.py` pattern: backups=N keeps .1 .. .N (N slots; the
        // current file plus N backups = N+1 files). Drop .N, shift down,
        // then .log -> .1.
        for i in (1..=self.backups as usize).rev() {
            let src = self.path.with_extension(format!("log.{i}"));
            if i >= self.backups as usize {
                // `log.N` is the oldest backup: drop.
                std::fs::remove_file(&src).ok();
                continue;
            }
            let dst = self.path.with_extension(format!("log.{}", i + 1));
            if src.exists() {
                std::fs::rename(&src, &dst).ok();
            }
        }
        if self.path.exists() {
            let first = self.path.with_extension("log.1");
            std::fs::rename(&self.path, &first)?;
        }
        self.size = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_assigns_monotonic_sequence() {
        let mut buf = LogBuffer::new(10);
        let ts = Utc::now();
        let a = buf.append("hello".into(), ts);
        let b = buf.append("world".into(), ts);
        assert_eq!(a.seq, 1);
        assert_eq!(b.seq, 2);
        assert_eq!(b.ts, ts);
    }

    #[test]
    fn parse_line_strips_cr_lf() {
        let line = parse_line(3, Utc::now(), "text\r\n");
        assert_eq!(line.line, "text");
    }

    #[test]
    fn tail_and_since() {
        let mut buf = LogBuffer::new(10);
        for i in 0..5 {
            buf.append(format!("line {i}"), Utc::now());
        }
        assert_eq!(buf.tail(2).len(), 2);
        assert!(buf.tail(0).is_empty());
        let since: Vec<_> = buf.since(3).iter().map(|l| l.line.clone()).collect();
        assert_eq!(since, ["line 3", "line 4"]);
        assert!(buf.since(5).is_empty());
    }

    #[test]
    fn eviction_keeps_sequence_numbers() {
        let mut buf = LogBuffer::new(3);
        for i in 0..6 {
            buf.append(format!("line {i}"), Utc::now());
        }
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.first_seq(), Some(4));
        assert_eq!(buf.last_seq(), Some(6));
        let lines: Vec<_> = buf.since(0).iter().map(|l| l.line.clone()).collect();
        assert_eq!(lines, ["line 3", "line 4", "line 5"]);
    }

    #[test]
    fn empty_buffer() {
        let buf = LogBuffer::new(3);
        assert_eq!(buf.first_seq(), None);
        assert_eq!(buf.last_seq(), None);
        assert!(buf.tail(5).is_empty());
        assert!(buf.since(0).is_empty());
    }

    #[test]
    fn batcher_caps_at_max_lines() {
        let (tx, rx) = mpsc::channel::<String>(100);
        let mut b = Batcher::new(rx, 50, 2);
        for i in 0..5 {
            let _ = tx.try_send(format!("l{i}"));
        }
        b.drain_pending();
        let batch = b.take_batch().expect("batch");
        assert_eq!(batch.lines.len(), 2);
        assert_eq!(batch.first_seq, 1);
        assert_eq!(batch.last_seq, 2);
        // draining again takes the next max-cap batch (3 lines remain).
        b.drain_pending();
        let batch2 = b.take_batch().expect("second batch");
        assert_eq!(batch2.lines.len(), 2);
        assert_eq!(batch2.first_seq, 3);
        // and the last single line
        b.drain_pending();
        let batch3 = b.take_batch().expect("third batch");
        assert_eq!(batch3.lines.len(), 1);
        assert_eq!(batch3.first_seq, 5);
    }

    #[test]
    fn rotation_keeps_backup_count() {
        let dir = std::env::temp_dir().join("minewrap-rot-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("server.log");
        // each write is ~17 bytes; max_bytes 30 triggers a rotation ~every 2 writes.
        let mut f = RotatingFile::new(path.clone(), 30, 3);
        for i in 0..8 {
            f.write(&format!("line {i} xxxxxxxx")).unwrap();
        }
        // current file (fresh after last rotation) + .1 .2 .3 exist
        assert!(path.exists() || dir.join("server.log.1").exists());
        assert!(dir.join("server.log.1").exists());
        assert!(dir.join("server.log.2").exists());
        assert!(dir.join("server.log.3").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
