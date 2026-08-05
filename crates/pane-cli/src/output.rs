//! Printing results.
//!
//! Two rules that exist for the benefit of scripted callers:
//!
//! * **No TTY sniffing.** Format is chosen by `--json` / `$PANE_FORMAT`, never
//!   by whether stdout is a terminal. Auto-switching means a human and an
//!   agent get different output from the same command, so documentation and
//!   bug reports stop matching what anyone actually sees — and an agent, always
//!   running through a pipe, would never see the documented form.
//! * **stdout is only ever the result.** Notes and errors go to stderr, so
//!   `pane captures body X | jq .` never has to strip anything.

use std::io::Write;

use pane_ipc::ApiError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Human,
    Json,
}

impl Format {
    pub fn resolve(json_flag: bool) -> Self {
        if json_flag {
            return Format::Json;
        }
        match std::env::var("PANE_FORMAT").as_deref() {
            Ok("json") | Ok("ndjson") => Format::Json,
            _ => Format::Human,
        }
    }
}

/// Exit codes. Stable contract — documented in `pane schema` and AGENTS.md,
/// and scripts branch on them.
pub mod exit {
    pub const OK: i32 = 0;
    pub const ERROR: i32 = 1;
    /// Reserved by clap for argument errors; never returned by hand.
    /// Referenced only by the test that asserts no ApiError kind maps onto it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub const USAGE: i32 = 2;
    pub const NOT_RUNNING: i32 = 3;
    pub const NO_DEVICE: i32 = 4;
    pub const NOT_FOUND: i32 = 5;
    pub const BAD_FILTER: i32 = 6;
    /// A deadline passed, or `--count` was not reached before `--timeout`.
    /// This is the assertion-failed code for scripted runs.
    pub const TIMEOUT: i32 = 7;
    pub const CONFLICT: i32 = 8;
}

/// Map an `ApiError::kind` onto an exit code.
pub fn exit_code_for_kind(kind: &str) -> i32 {
    use pane_ipc::kinds as k;
    match kind {
        k::PROXY_NOT_RUNNING | k::ENGINE_STOP => exit::NOT_RUNNING,
        k::TOOLING_MISSING
        | k::ADB
        | k::IOS_ADD_FAILED
        | k::ANDROID_ADD_FAILED
        | k::LOGCAT_SPAWN => exit::NO_DEVICE,
        k::NOT_FOUND => exit::NOT_FOUND,
        k::FILTER_PARSE => exit::BAD_FILTER,
        k::INVALID_ADDR | k::ENGINE_START => exit::CONFLICT,
        _ => exit::ERROR,
    }
}

/// Turn any error into an exit code, printing it in the right shape.
pub fn report_error(err: &anyhow::Error, format: Format) -> i32 {
    if let Some(api) = err.downcast_ref::<ApiError>() {
        match format {
            Format::Json => {
                let _ = writeln!(std::io::stderr(), "{}", serde_json::json!({ "error": api }));
            }
            Format::Human => {
                let _ = writeln!(std::io::stderr(), "pane: {}: {}", api.kind, api.message);
            }
        }
        return exit_code_for_kind(&api.kind);
    }

    // anyhow's Display shows only the outermost context, so `.context("opening
    // the Pane data directory")` would hide the cause underneath it — which is
    // where the actionable part lives. Print the whole chain.
    let chain: Vec<String> = err.chain().map(|c| c.to_string()).collect();
    let message = chain.join(": ");

    match format {
        Format::Json => {
            let _ = writeln!(
                std::io::stderr(),
                "{}",
                serde_json::json!({
                    "error": {
                        "kind": "error",
                        "message": message,
                        "details": { "chain": chain },
                    }
                })
            );
        }
        Format::Human => {
            let _ = writeln!(std::io::stderr(), "pane: {}", chain[0]);
            for cause in chain.iter().skip(1) {
                let _ = writeln!(std::io::stderr(), "  → {cause}");
            }
        }
    }
    // "no running instance" is a distinct, recoverable state and deserves its
    // own code even when it arrives as a plain anyhow error.
    if message.contains("needs a running Pane instance")
        || message.contains("no running Pane instance")
    {
        exit::NOT_RUNNING
    } else {
        exit::ERROR
    }
}

/// A note for the human, never on stdout.
pub fn note(msg: impl std::fmt::Display) {
    let _ = writeln!(std::io::stderr(), "pane: {msg}");
}

pub fn print_json(v: &serde_json::Value) {
    let out = std::io::stdout();
    let mut lock = out.lock();
    let _ = writeln!(
        lock,
        "{}",
        serde_json::to_string_pretty(v).unwrap_or_default()
    );
}

/// One NDJSON line, flushed immediately so a reader blocked on `read` wakes up
/// now rather than whenever the buffer happens to fill. Streaming consumers
/// depend on this — a scripted run reads the `ready` line before triggering
/// the app under test.
pub fn print_ndjson_line(v: &serde_json::Value) {
    let out = std::io::stdout();
    let mut lock = out.lock();
    let _ = writeln!(lock, "{}", serde_json::to_string(v).unwrap_or_default());
    let _ = lock.flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use pane_ipc::kinds as k;

    #[test]
    fn error_kinds_map_to_their_documented_codes() {
        assert_eq!(exit_code_for_kind(k::PROXY_NOT_RUNNING), exit::NOT_RUNNING);
        assert_eq!(exit_code_for_kind(k::NOT_FOUND), exit::NOT_FOUND);
        assert_eq!(exit_code_for_kind(k::FILTER_PARSE), exit::BAD_FILTER);
        assert_eq!(exit_code_for_kind(k::TOOLING_MISSING), exit::NO_DEVICE);
        assert_eq!(exit_code_for_kind(k::ENGINE_START), exit::CONFLICT);
        assert_eq!(exit_code_for_kind(k::DB), exit::ERROR);
    }

    /// clap owns exit code 2; returning it by hand would make a real failure
    /// indistinguishable from a typo in the command line.
    #[test]
    fn no_kind_maps_onto_claps_usage_code() {
        for kind in [
            k::DB,
            k::IO,
            k::NOT_FOUND,
            k::FILTER_PARSE,
            k::INVALID_ADDR,
            k::ENGINE_START,
            k::ENGINE_STOP,
            k::PROXY_NOT_RUNNING,
            k::NO_CA,
            k::ROTATE_FAILED,
            k::EXPORT_FAILED,
            k::WRITE,
            k::DECODE,
            k::REPLAY_FAILED,
            k::TOOLING_MISSING,
            k::ADB,
            k::IOS_ADD_FAILED,
            k::ANDROID_ADD_FAILED,
            k::REMOVE_FAILED,
            k::LOGCAT_SPAWN,
            k::WINDOW_BUILD,
            k::HOST_CAPTURE_ENABLE,
            k::HOST_CAPTURE_DISABLE,
        ] {
            assert_ne!(exit_code_for_kind(kind), exit::USAGE, "kind {kind}");
        }
    }

    #[test]
    fn json_format_comes_from_flag_or_env_but_never_from_a_tty_check() {
        assert_eq!(Format::resolve(true), Format::Json);
        // With no flag and no env var set in the test process, Human.
        std::env::remove_var("PANE_FORMAT");
        assert_eq!(Format::resolve(false), Format::Human);
    }
}
