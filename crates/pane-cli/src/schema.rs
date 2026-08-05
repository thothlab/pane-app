//! `pane schema` — the command surface as JSON.
//!
//! Hand-owned rather than derived from clap's help output: clap's rendering is
//! clap's to change between minor versions, whereas this is a contract callers
//! can parse and we can snapshot-test.

use serde_json::{json, Value};

pub fn schema() -> Value {
    json!({
        "pane_version": env!("CARGO_PKG_VERSION"),
        "schema_version": 1,
        "control_protocol": pane_control::PROTOCOL_VERSION,
        "notes": [
            "Set PANE_FORMAT=json once per session for machine-readable output.",
            "Every <selector> accepts a full id, a unique id prefix, or a name substring.",
            "Nothing prompts; destructive commands require --yes.",
        ],
        "exit_codes": {
            "0": "ok",
            "1": "error",
            "2": "usage (clap)",
            "3": "no running instance / proxy stopped",
            "4": "no device or missing adb",
            "5": "not found",
            "6": "bad filter",
            "7": "timeout, or --count not reached (assertion failed)",
            "8": "conflict (port in use, precondition failed)"
        },
        "dsl": {
            "captures": {
                "keys": {
                    "host": "server host, substring, * glob",
                    "path": "url path, substring, * glob",
                    "method": "exact, uppercased",
                    "status": "N or N..M",
                    "size": "N or N..M (total bytes)",
                    "duration": "N or N..M (ms)",
                    "mime": "response Content-Type substring",
                    "error": "error kind, exact",
                    "device": "device name or serial; __host__ for this Mac",
                    "state": "completed | stubbed | patched | error",
                    "rule": "rule that served a mocked response: name substring or exact id"
                },
                "operators": {
                    "bareword": "substring over host OR path",
                    "!term": "negation",
                    "a,b": "OR within one key",
                    "N..M": "inclusive range",
                    "\"quoted value\"": "keeps spaces"
                },
                "examples": [
                    "status:500..599 host:api.example.com",
                    "state:stubbed rule:orders-500",
                    "!state:stubbed"
                ]
            },
            "logcat": {
                "keys": {
                    "tag": "substring, case-insensitive",
                    "msg": "message substring",
                    "level": "V D I W E F S, or a range like W..F",
                    "pid": "numeric",
                    "app": "package name; resolved to pids by the CLI"
                },
                "operators": { "~regex": "regex over tag OR message", "bareword": "substring over tag OR message" }
            }
        },
        "ops": pane_control::dispatch::OPS,
        "commands": commands(),
    })
}

fn commands() -> Value {
    json!([
        cmd(
            "doctor",
            "Proxy, devices, adb and CA at a glance",
            &[],
            false
        ),
        cmd("schema", "This document", &[], false),
        cmd("install", "Symlink onto PATH as `pane`", &["--dir"], false),
        cmd("mcp", "Run as an MCP server over stdio", &[], false),
        cmd(
            "proxy start",
            "Start the MITM proxy",
            &["--host", "--port"],
            false
        ),
        cmd(
            "proxy stop",
            "Stop it and undo device/system proxy settings",
            &[],
            true
        ),
        cmd(
            "proxy status",
            "Running? where? how many captures",
            &[],
            false
        ),
        cmd(
            "proxy run",
            "Foreground headless instance with its own control socket",
            &["--host", "--port", "--no-proxy"],
            false
        ),
        cmd(
            "captures list",
            "Recent captures, oldest-first",
            &["--filter", "--limit", "--fields", "--full"],
            false
        ),
        cmd("captures count", "Bare integer", &["--filter"], false),
        cmd("captures get", "One capture with headers", &[], false),
        cmd(
            "captures body",
            "Request or response body",
            &["--res", "--req", "--max-bytes", "--out", "--base64"],
            false
        ),
        stream(
            "captures tail",
            "Completed captures as NDJSON",
            &["--filter", "--count", "--timeout"]
        ),
        cmd(
            "captures export",
            "Export as curl or HAR",
            &["--format", "--out"],
            false
        ),
        cmd("captures clear", "Delete every capture", &["--yes"], false),
        cmd("rules ls", "List rules", &[], false),
        cmd("rules get", "One rule", &[], false),
        cmd("rules enable", "Enable by name substring or id", &[], false),
        cmd(
            "rules disable",
            "Disable by name substring or id",
            &[],
            false
        ),
        cmd("rules rm", "Delete a rule", &["--yes"], false),
        cmd(
            "rules mock",
            "Create a stub rule",
            &[
                "--host",
                "--path",
                "--method",
                "--status",
                "--body",
                "--body-file",
                "--mime",
                "--delay-ms",
                "--name",
                "--disabled"
            ],
            false
        ),
        cmd(
            "rules from-capture",
            "Derive a rule from a real capture",
            &["--status", "--body", "--body-file", "--name"],
            false
        ),
        cmd(
            "rules import",
            "Import a pane-rules bundle",
            &["--dry-run"],
            false
        ),
        cmd(
            "rules export",
            "Export in the GUI-compatible pane-rules format",
            &["--out"],
            false
        ),
        cmd("devices ls", "Paired devices", &[], false),
        cmd(
            "devices attached",
            "Devices plugged in right now",
            &[],
            false
        ),
        cmd(
            "devices add",
            "Pair over USB; needs a running proxy",
            &["--platform"],
            true
        ),
        cmd("devices rm", "Unpair", &["--yes"], true),
        cmd(
            "logcat attach",
            "Start the adb logcat stream",
            &["--serial"],
            true
        ),
        cmd("logcat detach", "Stop it", &["--serial"], true),
        cmd(
            "logcat query",
            "Query persisted rows",
            &["--serial", "--filter", "--limit"],
            false
        ),
        cmd("logcat pids", "PID to process name", &["--serial"], false),
        cmd(
            "logcat clear",
            "Delete rows for a device",
            &["--serial", "--yes"],
            false
        ),
        cmd("ca show", "Certificate details", &[], false),
        cmd(
            "ca export",
            "Export the root certificate",
            &["--format", "--out"],
            false
        ),
    ])
}

fn cmd(path: &str, about: &str, flags: &[&str], needs_instance: bool) -> Value {
    json!({ "path": path, "about": about, "flags": flags, "requires_instance": needs_instance, "stream": false })
}

fn stream(path: &str, about: &str, flags: &[&str]) -> Value {
    json!({
        "path": path, "about": about, "flags": flags,
        "requires_instance": true, "stream": true, "format": "ndjson",
        "line_kinds": ["ready", "capture", "error", "end"],
        "contract": "first line is always `ready`; last is always `end`. Read `ready` before triggering the app under test."
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_self_consistent() {
        let s = schema();
        assert!(s["commands"].as_array().unwrap().len() > 20);
        assert_eq!(
            s["exit_codes"]["7"],
            "timeout, or --count not reached (assertion failed)"
        );
        // `state:` and `rule:` are the keys the whole verify-the-mock workflow
        // rests on; if they vanish from the schema the skills mislead callers.
        assert!(s["dsl"]["captures"]["keys"]["state"].is_string());
        assert!(s["dsl"]["captures"]["keys"]["rule"].is_string());
    }

    #[test]
    fn tail_is_documented_as_a_stream_with_the_ready_contract() {
        let s = schema();
        let tail = s["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["path"] == "captures tail")
            .expect("captures tail present");
        assert_eq!(tail["stream"], true);
        assert_eq!(tail["format"], "ndjson");
        assert!(tail["contract"].as_str().unwrap().contains("ready"));
    }
}
