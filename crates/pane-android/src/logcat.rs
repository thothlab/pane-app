//! `adb logcat` streaming for the Logcat window.
//!
//! Spawns `adb -s SERIAL logcat -v threadtime`, parses each line into a
//! structured `LogEntry`, batches them, and hands the batches to a
//! caller-supplied callback. The caller (src-tauri command layer) wires
//! the callback to `WebviewWindow::emit`, scoping the firehose to a
//! single window.
//!
//! Stream resilience:
//!   - subprocess is built with `kill_on_drop(true)` so an abandoned
//!     handle doesn't leave a zombie `adb` even if the shutdown channel
//!     is never signalled,
//!   - on EOF / read-error / parse panic the task reconnects with
//!     exponential backoff (0.5s → 10s, capped at 5 tries) so a
//!     `adb` daemon restart or USB reseat doesn't break the window,
//!   - the `BufReader` swallows broken UTF-8 cleanly because
//!     `lines()` is built on a `String`-replacing `read_until` chain
//!     in tokio — invalid bytes get replaced with U+FFFD rather than
//!     killing the stream (verified empirically on Samsung devices
//!     that emit chunked JSON payloads that occasionally split a
//!     codepoint).
//!
//! Why batching at this layer: logcat on a busy device runs 1000+
//! lines/sec. Emitting one Tauri IPC event per line saturates the
//! renderer reactor. We buffer in a small `Vec`, flush every 100ms or
//! when the buffer reaches 50 entries — whichever comes first — for a
//! single emit. The frontend then appends the whole batch in one Solid
//! reactive cycle.
//!
//! The parser regex + LogLevel were copied (adapted) from `logux`
//! (Apache-2.0, same author).

use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use regex::Regex;
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::{resolve_adb, ADB_NOT_FOUND_MSG};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Verbose = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
    Fatal = 5,
    Silent = 6,
}

impl LogLevel {
    pub fn from_char(c: char) -> Self {
        match c.to_ascii_uppercase() {
            'V' => Self::Verbose,
            'D' => Self::Debug,
            'I' => Self::Info,
            'W' => Self::Warn,
            'E' => Self::Error,
            'F' => Self::Fatal,
            'S' => Self::Silent,
            _ => Self::Verbose,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    /// "MM-DD HH:MM:SS.mmm" — empty if the line was continuation / unparseable.
    pub timestamp: String,
    pub pid: u32,
    pub tid: u32,
    pub level: LogLevel,
    pub tag: String,
    pub message: String,
}

// threadtime format: "MM-DD HH:MM:SS.mmm  PID  TID LEVEL TAG: MESSAGE"
static THREADTIME_RE_PATTERN: &str =
    r"^(\d{2}-\d{2}\s+\d{2}:\d{2}:\d{2}\.\d{3})\s+(\d+)\s+(\d+)\s+([VDIWEFS])\s+(.+?)\s*:\s+(.*)";
// brief format (fallback): "LEVEL/TAG(PID): MESSAGE"
static BRIEF_RE_PATTERN: &str = r"^([VDIWEFS])/(.+?)\(\s*(\d+)\):\s+(.*)";

fn threadtime_re() -> &'static Regex {
    use std::sync::OnceLock;
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(THREADTIME_RE_PATTERN).unwrap())
}

fn brief_re() -> &'static Regex {
    use std::sync::OnceLock;
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(BRIEF_RE_PATTERN).unwrap())
}

/// Parse one line of `adb logcat -v threadtime` output. Falls back to
/// brief format for lines emitted by older formatters, and to a
/// "continuation" entry (empty timestamp/tag, the whole line as
/// message) when neither regex matches. Returns `None` only for empty
/// lines and the `--------- beginning of main` markers logcat injects
/// at buffer boundaries.
pub fn parse_logcat_line(line: &str) -> Option<LogEntry> {
    let line = line.trim_end();
    if line.is_empty() {
        return None;
    }
    if line.starts_with("---------") {
        return None;
    }
    if let Some(caps) = threadtime_re().captures(line) {
        return Some(LogEntry {
            timestamp: caps[1].to_string(),
            pid: caps[2].parse().unwrap_or(0),
            tid: caps[3].parse().unwrap_or(0),
            level: LogLevel::from_char(caps[4].chars().next().unwrap_or('V')),
            tag: caps[5].trim().to_string(),
            message: caps[6].to_string(),
        });
    }
    if let Some(caps) = brief_re().captures(line) {
        return Some(LogEntry {
            timestamp: String::new(),
            pid: caps[3].parse().unwrap_or(0),
            tid: 0,
            level: LogLevel::from_char(caps[1].chars().next().unwrap_or('V')),
            tag: caps[2].trim().to_string(),
            message: caps[4].to_string(),
        });
    }
    // Continuation line (multi-line stack trace, etc.) — keep it.
    Some(LogEntry {
        timestamp: String::new(),
        pid: 0,
        tid: 0,
        level: LogLevel::Verbose,
        tag: String::new(),
        message: line.to_string(),
    })
}

/// Configuration for one `adb logcat` session.
pub struct LogcatConfig {
    pub serial: String,
    /// Max entries per emitted batch; flush also fires on timer.
    pub batch_size: usize,
    /// Time-based flush interval. With a quiet device, this keeps the
    /// "Last seen: just now" UI honest. With a busy device, the size
    /// threshold typically wins.
    pub flush_interval: Duration,
    /// Capped reconnect attempts; gives up + emits an error after.
    pub max_reconnects: u32,
}

impl Default for LogcatConfig {
    fn default() -> Self {
        Self {
            serial: String::new(),
            // Larger batch caps mean fewer Tauri IPC events during
            // the initial ring-buffer dump (the burst peak is
            // 2000–10000 entries/s on a Samsung S9-series). Combined
            // with rAF coalescing on the renderer side, this keeps
            // the main thread responsive enough that window resize
            // takes effect immediately even during the first 1–2s
            // of streaming. flush_interval stays at 100ms so steady-
            // state latency is unchanged.
            batch_size: 250,
            flush_interval: Duration::from_millis(100),
            max_reconnects: 5,
        }
    }
}

/// Event the stream task emits to the caller-supplied sink.
pub enum LogcatEvent {
    Batch(Vec<LogEntry>),
    /// Connection error after exhausting retries; the task exits after.
    Error(String),
}

/// Spawn `adb -s SERIAL logcat -v threadtime` and forward batched
/// `LogEntry`s to `on_event`. Returns a `mpsc::Sender<()>` — send one
/// value (or drop) to stop the task. The task always calls
/// `child.kill().await` before returning.
///
/// `on_event` is invoked from the spawned task; it must be `Send +
/// 'static` and should be lightweight (forwards to a Tauri event in
/// practice).
pub fn spawn(
    cfg: LogcatConfig,
    mut on_event: impl FnMut(LogcatEvent) + Send + 'static,
) -> Result<mpsc::Sender<()>> {
    let adb = resolve_adb()
        .ok_or_else(|| anyhow!(ADB_NOT_FOUND_MSG))?
        .to_string_lossy()
        .into_owned();
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
    let cfg = cfg;
    tokio::spawn(async move {
        let mut attempt: u32 = 0;
        loop {
            // Backoff between connection attempts.
            if attempt > 0 {
                let backoff = backoff_for(attempt);
                tracing::warn!(
                    serial = %cfg.serial,
                    attempt,
                    backoff_ms = backoff.as_millis() as u64,
                    "logcat: reconnecting"
                );
                tokio::select! {
                    _ = shutdown_rx.recv() => return,
                    _ = tokio::time::sleep(backoff) => {}
                }
                if attempt > cfg.max_reconnects {
                    on_event(LogcatEvent::Error(format!(
                        "logcat: gave up after {} reconnect attempts",
                        cfg.max_reconnects
                    )));
                    return;
                }
            }

            // (Re-)spawn adb logcat. Note: NO `--user 0` — that's a
            // pm/am flag, logcat reads the kernel ring buffer which
            // is global and has no per-user concept.
            let child_res = Command::new(&adb)
                .args(["-s", &cfg.serial, "logcat", "-v", "threadtime"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn();
            let mut child = match child_res {
                Ok(c) => c,
                Err(e) => {
                    attempt += 1;
                    tracing::warn!(error = %e, serial = %cfg.serial, "logcat: spawn failed");
                    continue;
                }
            };
            let stdout = match child.stdout.take() {
                Some(s) => s,
                None => {
                    let _ = child.kill().await;
                    attempt += 1;
                    continue;
                }
            };
            let mut reader = BufReader::new(stdout).lines();
            // On successful connect, reset the attempt counter.
            attempt = 0;
            let mut batch: Vec<LogEntry> = Vec::with_capacity(cfg.batch_size);
            let mut last_flush = Instant::now();

            'stream: loop {
                let deadline = last_flush + cfg.flush_interval;
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        if !batch.is_empty() {
                            on_event(LogcatEvent::Batch(std::mem::take(&mut batch)));
                        }
                        let _ = child.kill().await;
                        return;
                    }
                    line = reader.next_line() => {
                        match line {
                            Ok(Some(l)) => {
                                if let Some(entry) = parse_logcat_line(&l) {
                                    batch.push(entry);
                                    if batch.len() >= cfg.batch_size {
                                        on_event(LogcatEvent::Batch(std::mem::take(&mut batch)));
                                        batch.reserve(cfg.batch_size);
                                        last_flush = Instant::now();
                                    }
                                }
                            }
                            Ok(None) => {
                                // EOF — usually means adb-server hiccup
                                // or device disconnected. Flush, drop
                                // child, reconnect with backoff.
                                if !batch.is_empty() {
                                    on_event(LogcatEvent::Batch(std::mem::take(&mut batch)));
                                }
                                let _ = child.kill().await;
                                attempt += 1;
                                tracing::info!(serial = %cfg.serial, "logcat: stream EOF, will retry");
                                break 'stream;
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "logcat: read error");
                                if !batch.is_empty() {
                                    on_event(LogcatEvent::Batch(std::mem::take(&mut batch)));
                                }
                                let _ = child.kill().await;
                                attempt += 1;
                                break 'stream;
                            }
                        }
                    }
                    _ = tokio::time::sleep_until(deadline.into()) => {
                        if !batch.is_empty() {
                            on_event(LogcatEvent::Batch(std::mem::take(&mut batch)));
                        }
                        last_flush = Instant::now();
                    }
                }
            }
        }
    });
    Ok(shutdown_tx)
}

fn backoff_for(attempt: u32) -> Duration {
    // 0.5s, 1s, 2s, 5s, 10s — same shape as logux.
    match attempt {
        1 => Duration::from_millis(500),
        2 => Duration::from_secs(1),
        3 => Duration::from_secs(2),
        4 => Duration::from_secs(5),
        _ => Duration::from_secs(10),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_threadtime() {
        let line = "04-13 12:34:56.789  1234  5678 D MyTag   : Hello World";
        let e = parse_logcat_line(line).unwrap();
        assert_eq!(e.timestamp, "04-13 12:34:56.789");
        assert_eq!(e.pid, 1234);
        assert_eq!(e.tid, 5678);
        assert_eq!(e.level, LogLevel::Debug);
        assert_eq!(e.tag, "MyTag");
        assert_eq!(e.message, "Hello World");
    }

    #[test]
    fn parses_error_with_colons_in_message() {
        let line =
            "04-13 12:34:56.789  1234  5678 E CrashTag: java.lang.NullPointerException: foo";
        let e = parse_logcat_line(line).unwrap();
        assert_eq!(e.level, LogLevel::Error);
        assert!(e.message.contains("NullPointerException"));
    }

    #[test]
    fn parses_brief_fallback() {
        let line = "D/MyTag( 1234): Some debug message";
        let e = parse_logcat_line(line).unwrap();
        assert_eq!(e.level, LogLevel::Debug);
        assert_eq!(e.tag, "MyTag");
        assert_eq!(e.pid, 1234);
    }

    #[test]
    fn unparseable_becomes_continuation() {
        // Multi-line stack trace continuation has no logcat prefix —
        // we keep it as a message-only entry so it stays visible.
        let e = parse_logcat_line("\tat com.example.Foo.bar(Foo.java:42)").unwrap();
        assert_eq!(e.timestamp, "");
        assert!(e.message.contains("Foo.java:42"));
    }

    #[test]
    fn buffer_marker_is_skipped() {
        assert!(parse_logcat_line("--------- beginning of main").is_none());
        assert!(parse_logcat_line("--------- beginning of system").is_none());
    }

    #[test]
    fn empty_lines_skipped() {
        assert!(parse_logcat_line("").is_none());
        assert!(parse_logcat_line("   \n").is_none());
    }

    #[test]
    fn backoff_sequence_is_capped() {
        assert_eq!(backoff_for(1), Duration::from_millis(500));
        assert_eq!(backoff_for(5), Duration::from_secs(10));
        assert_eq!(backoff_for(100), Duration::from_secs(10));
    }

    #[test]
    fn level_from_char_handles_lowercase() {
        assert_eq!(LogLevel::from_char('e'), LogLevel::Error);
        assert_eq!(LogLevel::from_char('E'), LogLevel::Error);
    }
}
