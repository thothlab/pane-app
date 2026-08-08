//! `pane mcp` — expose Pane to an agent as MCP tools over stdio.
//!
//! Hand-rolled JSON-RPC 2.0 rather than an SDK: the surface needed is small
//! (`initialize`, `tools/list`, `tools/call`), and this workspace pins rustc
//! to 1.94.1 with a documented history of a semver-compatible dependency
//! re-resolve breaking every platform build. Hand-rolled protocols are also
//! the house style here — see `pac.rs`, `heartbeat.rs`, `pane-setup-server`.
//!
//! Tools are shaped for the task, not mapped 1:1 onto CLI commands. The one
//! that matters most is `pane_captures_wait`: MCP has no streaming, so `tail`
//! becomes a blocking call that returns the captures it saw.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::output::exit;
use crate::session::Session;

const PROTOCOL_VERSION: &str = "2024-11-05";

pub async fn serve(data_dir: PathBuf) -> Result<i32> {
    let mut stdin = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = stdin.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                write(&mut stdout, &err_frame(Value::Null, -32700, &e.to_string())).await?;
                continue;
            }
        };

        // Notifications carry no id and must not be answered at all.
        let id = req.get("id").cloned();
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        if id.is_none() {
            continue;
        }
        let id = id.unwrap();

        let response = match method {
            "initialize" => ok_frame(
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "pane", "version": env!("CARGO_PKG_VERSION") }
                }),
            ),
            "tools/list" => ok_frame(id, json!({ "tools": tools() })),
            "tools/call" => {
                let params = req.get("params").cloned().unwrap_or(Value::Null);
                match call_tool(&data_dir, &params).await {
                    Ok(text) => ok_frame(
                        id,
                        json!({ "content": [{ "type": "text", "text": text }], "isError": false }),
                    ),
                    // Tool failures are reported in-band with isError, not as
                    // JSON-RPC errors: the model should see and react to them
                    // rather than the transport treating them as fatal.
                    Err(e) => ok_frame(
                        id,
                        json!({ "content": [{ "type": "text", "text": e.to_string() }], "isError": true }),
                    ),
                }
            }
            other => err_frame(id, -32601, &format!("unknown method `{other}`")),
        };
        write(&mut stdout, &response).await?;
    }
    Ok(exit::OK)
}

async fn write(out: &mut tokio::io::Stdout, v: &Value) -> Result<()> {
    out.write_all(serde_json::to_string(v)?.as_bytes()).await?;
    out.write_all(b"\n").await?;
    out.flush().await?;
    Ok(())
}

fn ok_frame(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn err_frame(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn tool(name: &str, description: &str, props: Value, required: Vec<&str>) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": props,
            "required": required,
        }
    })
}

pub fn tools() -> Vec<Value> {
    let filter_doc = "Captures filter DSL, identical to the GUI search bar. Keys: host: path: \
                      method: status: mime: size: duration: error: device: state: rule:. A bare \
                      word matches host OR path. `a,b` = OR, `N..M` = range, `!` negates. \
                      state is completed|stubbed|patched|error; rule: matches the rule that \
                      served a mocked response.";
    vec![
        tool("pane_doctor", "Proxy state, paired/attached devices, adb availability and CA fingerprint. Run this first.", json!({}), vec![]),
        tool(
            "pane_captures_list",
            "Recent captures, newest N returned oldest-first. Returns a compact summary per row including `state` and `matched_rule_name`.",
            json!({
                "filter": { "type": "string", "description": filter_doc },
                "limit": { "type": "integer", "description": "Default 50, max 2000." }
            }),
            vec![],
        ),
        tool(
            "pane_captures_count",
            "Number of captures matching a filter. The cheapest way to assert an outcome.",
            json!({ "filter": { "type": "string", "description": filter_doc } }),
            vec![],
        ),
        tool("pane_capture_get", "One capture with request and response headers.", json!({ "id": { "type": "string" } }), vec!["id"]),
        tool(
            "pane_capture_body",
            "Request or response body, decoded. Truncated to max_bytes (default 8192) to protect the context budget.",
            json!({
                "id": { "type": "string" },
                "side": { "type": "string", "enum": ["request", "response"] },
                "max_bytes": { "type": "integer" }
            }),
            vec!["id"],
        ),
        tool(
            "pane_captures_wait",
            "Block until `count` captures match `filter`, or `timeout_sec` elapses. MCP cannot stream, so this is the equivalent of `pane captures tail --count N --timeout D`: call it, then trigger the app under test. Reports whether the count was met.",
            json!({
                "filter": { "type": "string", "description": filter_doc },
                "count": { "type": "integer", "description": "Default 1." },
                "timeout_sec": { "type": "integer", "description": "Default 30." }
            }),
            vec![],
        ),
        tool("pane_captures_clear", "Delete every capture. Use between scenarios so assertions cannot match a previous run.", json!({}), vec![]),
        tool(
            "pane_collections_list",
            "Rule collections with their enabled state and rule count. A collection groups the rules for one scenario, so listing these is usually the right first step before switching scenarios.",
            json!({}),
            vec![],
        ),
        tool(
            "pane_collection_set_enabled",
            "Enable or disable a whole collection by name substring or id — switches every rule in that scenario at once.",
            json!({ "selector": { "type": "string" }, "enabled": { "type": "boolean" } }),
            vec!["selector", "enabled"],
        ),
        tool(
            "pane_collection_only",
            "Enable exactly one collection and disable all the others. The usual way to move from one scenario to the next without leaving the previous scenario's rules live and shadowing it.",
            json!({ "selector": { "type": "string" } }),
            vec!["selector"],
        ),
        tool("pane_rules_list", "All mock rules with their enabled state, matchers and response status. Use it to find the selector for pane_rule_set_enabled.", json!({}), vec![]),
        tool(
            "pane_rule_set_enabled",
            "Enable or disable a rule by name substring or id. Lets one compact rule set cover many scenarios instead of encoding the variant into every request body.",
            json!({ "selector": { "type": "string" }, "enabled": { "type": "boolean" } }),
            vec!["selector", "enabled"],
        ),
        tool(
            "pane_rules_set_enabled_bulk",
            "Enable or disable many rules at once: the whole library (scope 'all'), one collection (scope 'collection' + collection selector), or the rules in no collection (scope 'ungrouped'). Use this to reset to a known state before a run — 'disable everything, then enable the one collection I want' — instead of toggling rules one at a time.",
            json!({
                "enabled": { "type": "boolean" },
                "scope": { "type": "string", "enum": ["all", "collection", "ungrouped"] },
                "collection": { "type": "string", "description": "Collection name substring or id. Required when scope is 'collection'." }
            }),
            vec!["enabled", "scope"],
        ),
        tool(
            "pane_collection_delete",
            "Delete a collection. Its rules survive and move to Ungrouped; they are not deleted.",
            json!({ "selector": { "type": "string" } }),
            vec!["selector"],
        ),
        tool(
            "pane_rule_mock",
            "Create a stub rule that answers matching requests with a fixed response.",
            json!({
                "host": { "type": "string" },
                "path": { "type": "string" },
                "method": { "type": "string" },
                "status": { "type": "integer" },
                "body": { "type": "string" },
                "name": { "type": "string" }
            }),
            vec!["host"],
        ),
        tool("pane_devices_list", "Devices already paired with Pane, with their state and whether the CA is installed. Traffic only flows through Pane for devices listed here.", json!({}), vec![]),
        tool("pane_devices_attached", "Devices plugged in over USB right now that could be paired, whether or not Pane knows them yet. Run before pane_device_add to get the serial.", json!({}), vec![]),
        tool(
            "pane_device_add",
            "Pair a device over USB and route its traffic through Pane. Requires a running proxy.",
            json!({ "serial": { "type": "string" }, "platform": { "type": "string", "enum": ["android", "ios"] } }),
            vec!["serial"],
        ),
        tool(
            "pane_logcat_query",
            "Android log lines for a device. Filter keys: tag: msg: level: pid: app:, plus ~regex.",
            json!({
                "serial": { "type": "string" },
                "filter": { "type": "string" },
                "limit": { "type": "integer" }
            }),
            vec!["serial"],
        ),
        tool("pane_proxy_status", "Is the proxy running, on what address, with how many captures.", json!({}), vec![]),
    ]
}

async fn call_tool(data_dir: &Path, params: &Value) -> Result<String> {
    let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let mut s = Session::open(data_dir.to_path_buf()).await?;

    let pretty = |v: Value| -> Result<String> { Ok(serde_json::to_string_pretty(&v)?) };

    match name {
        "pane_doctor" => {
            let out = json!({
                "proxy": s.call("proxy.status", Value::Null).await.unwrap_or(Value::Null),
                "android_tooling": s.call("devices.tooling_status", Value::Null).await.unwrap_or(Value::Null),
                "devices_paired": s.call("devices.list", Value::Null).await.unwrap_or(json!([])),
                "devices_attached": s.call("devices.attached", Value::Null).await.unwrap_or(json!([])),
                "ca": s.call("ca.current", Value::Null).await.unwrap_or(Value::Null),
            });
            pretty(out)
        }
        "pane_proxy_status" => pretty(s.call("proxy.status", Value::Null).await?),
        "pane_captures_list" => {
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(50);
            pretty(
                s.call(
                    "captures.list",
                    json!({ "filter": args.get("filter"), "limit": limit, "before": null }),
                )
                .await?,
            )
        }
        "pane_captures_count" => {
            let v = s
                .call(
                    "captures.list",
                    json!({ "filter": args.get("filter"), "limit": 2000, "before": null }),
                )
                .await?;
            Ok(v.as_array().map(|a| a.len()).unwrap_or(0).to_string())
        }
        "pane_capture_get" => pretty(
            s.call(
                "captures.get",
                args.get("id").cloned().unwrap_or(Value::Null),
            )
            .await?,
        ),
        "pane_capture_body" => {
            let id = args.get("id").cloned().unwrap_or(Value::Null);
            let cap = s.call("captures.get", id).await?;
            let field = match args.get("side").and_then(|v| v.as_str()) {
                Some("request") => "req_body_id",
                _ => "res_body_id",
            };
            let Some(body_id) = cap.get(field).and_then(|v| v.as_str()) else {
                return Ok("(this capture has no body on that side)".into());
            };
            let max = args
                .get("max_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(8192);
            let body = s
                .call(
                    "captures.body",
                    json!({ "body_id": body_id, "max_bytes": max }),
                )
                .await?;
            let b64 = body["bytes_base64"].as_str().unwrap_or("");
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::STANDARD.decode(b64)?;
            let mut text = String::from_utf8_lossy(&bytes).into_owned();
            if body["truncated"].as_bool().unwrap_or(false) {
                text.push_str(&format!(
                    "\n\n[truncated at {} of {} bytes]",
                    bytes.len(),
                    body["total_size"].as_u64().unwrap_or(0)
                ));
            }
            Ok(text)
        }
        "pane_captures_clear" => pretty(s.call("captures.clear", json!({})).await?),
        "pane_captures_wait" => {
            let filter = args
                .get("filter")
                .and_then(|v| v.as_str())
                .map(String::from);
            let want = args.get("count").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
            let secs = args
                .get("timeout_sec")
                .and_then(|v| v.as_u64())
                .unwrap_or(30);
            wait_for_captures(&mut s, filter, want, secs).await
        }
        "pane_collections_list" => pretty(s.call("collections.list", Value::Null).await?),
        "pane_collection_set_enabled" => {
            let selector = args.get("selector").and_then(|v| v.as_str()).unwrap_or("");
            let enabled = args
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let all = s.call("collections.list", Value::Null).await?;
            let id = pick_named(&all, selector, "collection")?;
            s.call(
                "collections.set_enabled",
                json!({ "id": id, "enabled": enabled }),
            )
            .await?;
            Ok(format!(
                "{} collection {id}",
                if enabled { "enabled" } else { "disabled" }
            ))
        }
        "pane_collection_only" => {
            let selector = args.get("selector").and_then(|v| v.as_str()).unwrap_or("");
            let all = s.call("collections.list", Value::Null).await?;
            let keep = pick_named(&all, selector, "collection")?;
            let empty = vec![];
            let mut changed = 0usize;
            for c in all.as_array().unwrap_or(&empty) {
                let id = c["id"].as_str().unwrap_or("").to_string();
                let want = id == keep;
                if c["enabled"].as_bool().unwrap_or(false) == want {
                    continue;
                }
                s.call(
                    "collections.set_enabled",
                    json!({ "id": id, "enabled": want }),
                )
                .await?;
                changed += 1;
            }
            Ok(format!(
                "only `{selector}` is enabled now ({changed} collection(s) changed)"
            ))
        }
        "pane_rules_list" => pretty(s.call("rules.list", Value::Null).await?),
        "pane_rule_set_enabled" => {
            let selector = args.get("selector").and_then(|v| v.as_str()).unwrap_or("");
            let enabled = args
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let rules = s.call("rules.list", Value::Null).await?;
            let id = pick_named(&rules, selector, "rule")?;
            s.call("rules.set_enabled", json!({ "id": id, "enabled": enabled }))
                .await?;
            Ok(format!(
                "{} rule {id}",
                if enabled { "enabled" } else { "disabled" }
            ))
        }
        "pane_rules_set_enabled_bulk" => {
            let enabled = args
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let scope = match args.get("scope").and_then(|v| v.as_str()).unwrap_or("") {
                "all" => json!({ "kind": "all" }),
                "ungrouped" => json!({ "kind": "ungrouped" }),
                "collection" => {
                    let sel = args
                        .get("collection")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if sel.is_empty() {
                        return Err(anyhow!("scope 'collection' needs a `collection` selector"));
                    }
                    let all = s.call("collections.list", Value::Null).await?;
                    json!({ "kind": "collection", "id": pick_named(&all, sel, "collection")? })
                }
                other => {
                    return Err(anyhow!(
                        "unknown scope `{other}` — use all, collection or ungrouped"
                    ))
                }
            };
            let v = s
                .call(
                    "rules.set_enabled_bulk",
                    json!({ "enabled": enabled, "scope": scope }),
                )
                .await?;
            let matched = v.get("matched").and_then(|x| x.as_u64()).unwrap_or(0);
            let changed = v.get("changed").and_then(|x| x.as_u64()).unwrap_or(0);
            Ok(format!(
                "{} {changed} of {matched} rules",
                if enabled { "enabled" } else { "disabled" }
            ))
        }
        "pane_collection_delete" => {
            let selector = args.get("selector").and_then(|v| v.as_str()).unwrap_or("");
            let all = s.call("collections.list", Value::Null).await?;
            let id = pick_named(&all, selector, "collection")?;
            s.call("collections.delete", json!(id)).await?;
            Ok(format!(
                "deleted collection {id}; its rules moved to Ungrouped"
            ))
        }
        "pane_rule_mock" => {
            let host = args.get("host").and_then(|v| v.as_str()).unwrap_or("");
            let body_b64 = args.get("body").and_then(|v| v.as_str()).map(|b| {
                use base64::Engine as _;
                base64::engine::general_purpose::STANDARD.encode(b.as_bytes())
            });
            pretty(
                s.call(
                    "rules.upsert",
                    json!({
                        "id": null,
                        "name": args.get("name").and_then(|v| v.as_str()).unwrap_or(host),
                        "enabled": true, "priority": 0, "collection_id": null,
                        "mode": "stub", "patches": [],
                        "match_host_glob": host,
                        "match_method": args.get("method"),
                        "match_path_glob": args.get("path"),
                        "match_params": [], "match_req_body": null, "match_conditions": [],
                        "res_status": args.get("status").and_then(|v| v.as_u64()).unwrap_or(200),
                        "res_headers": [{ "name": "content-type", "value": "application/json" }],
                        "res_body_id": null,
                        "res_body_base64": body_b64,
                        "res_body_mime": "application/json",
                        "res_delay_ms": 0,
                    }),
                )
                .await?,
            )
        }
        "pane_devices_list" => pretty(s.call("devices.list", Value::Null).await?),
        "pane_devices_attached" => pretty(s.call("devices.attached", Value::Null).await?),
        "pane_device_add" => {
            let serial = args.get("serial").and_then(|v| v.as_str()).unwrap_or("");
            let op = match args.get("platform").and_then(|v| v.as_str()) {
                Some("ios") => "devices.add_ios",
                _ => "devices.add_android",
            };
            pretty(s.call(op, json!({ "serial": serial })).await?)
        }
        "pane_logcat_query" => {
            let serial = args.get("serial").and_then(|v| v.as_str()).unwrap_or("");
            let filter = args
                .get("filter")
                .and_then(|v| v.as_str())
                .map(String::from);
            let (include, exclude) =
                crate::logcat_app::resolve_app_pids(&mut s, serial, filter.as_deref()).await?;
            pretty(
                s.call(
                    "logcat.query",
                    json!({
                        "serial": serial, "filter": filter,
                        "include_pids": include, "exclude_pids": exclude,
                        "limit": args.get("limit").and_then(|v| v.as_u64()).unwrap_or(200),
                    }),
                )
                .await?,
            )
        }
        other => anyhow::bail!("unknown tool `{other}`"),
    }
}

/// Poll for matching captures.
///
/// Polling rather than subscribing keeps this working in direct mode (no
/// running instance to subscribe to). The trade-off is a bounded miss window
/// of one interval, which is acceptable because the caller triggers the app
/// *after* this starts.
async fn wait_for_captures(
    s: &mut Session,
    filter: Option<String>,
    want: usize,
    timeout_sec: u64,
) -> Result<String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_sec);
    let baseline = matching(s, &filter).await?;

    loop {
        let rows = matching(s, &filter).await?;
        let fresh: Vec<Value> = rows
            .into_iter()
            .filter(|r| !baseline.iter().any(|b| b["id"] == r["id"]))
            .collect();
        if fresh.len() >= want {
            return Ok(serde_json::to_string_pretty(&json!({
                "matched": fresh.len(),
                "wanted": want,
                "timed_out": false,
                "captures": fresh,
            }))?);
        }
        if std::time::Instant::now() >= deadline {
            return Ok(serde_json::to_string_pretty(&json!({
                "matched": fresh.len(),
                "wanted": want,
                "timed_out": true,
                "captures": fresh,
                "hint": "Nothing matched in time. Check the proxy is running, the device is \
                         paired, and the filter is right — `pane_captures_list` with no filter \
                         shows whether any traffic arrived at all.",
            }))?);
        }
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    }
}

async fn matching(s: &mut Session, filter: &Option<String>) -> Result<Vec<Value>> {
    let v = s
        .call(
            "captures.list",
            json!({ "filter": filter, "limit": 2000, "before": null }),
        )
        .await?;
    Ok(v.as_array().cloned().unwrap_or_default())
}

fn pick_named(rules: &Value, selector: &str, what: &str) -> Result<String> {
    let empty = vec![];
    let list = rules.as_array().unwrap_or(&empty);
    let sel = selector.to_lowercase();
    let hits: Vec<&Value> = list
        .iter()
        .filter(|r| {
            r["id"]
                .as_str()
                .map(|i| i.eq_ignore_ascii_case(selector))
                .unwrap_or(false)
                || r["id"]
                    .as_str()
                    .map(|i| i.to_lowercase().starts_with(&sel))
                    .unwrap_or(false)
                || r["name"]
                    .as_str()
                    .map(|n| n.to_lowercase().contains(&sel))
                    .unwrap_or(false)
        })
        .collect();
    match hits.len() {
        0 => anyhow::bail!("no {what} matches `{selector}`"),
        1 => Ok(hits[0]["id"].as_str().unwrap_or("").to_string()),
        n => anyhow::bail!(
            "`{selector}` matches {n} {what}s: {}. Use a longer substring or an id.",
            hits.iter()
                .filter_map(|r| r["name"].as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_schema_and_a_description() {
        for t in tools() {
            assert!(t["name"].as_str().is_some_and(|n| n.starts_with("pane_")));
            assert!(
                t["description"].as_str().unwrap_or("").len() > 20,
                "tool {} needs a description an agent can select on",
                t["name"]
            );
            assert_eq!(t["inputSchema"]["type"], "object");
        }
    }

    /// MCP has no streaming, so the tail equivalent must exist as a blocking
    /// call — without it an agent has no way to await traffic at all.
    #[test]
    fn the_streaming_gap_is_covered_by_a_wait_tool() {
        let names: Vec<String> = tools()
            .iter()
            .map(|t| t["name"].as_str().unwrap_or("").to_string())
            .collect();
        assert!(names.contains(&"pane_captures_wait".to_string()));
        // The five the auto-run brief called the minimum useful set.
        for required in [
            "pane_rule_set_enabled",
            "pane_captures_list",
            "pane_captures_clear",
            "pane_device_add",
        ] {
            assert!(names.contains(&required.to_string()), "missing {required}");
        }
    }

    #[test]
    fn ambiguous_rule_selector_is_refused() {
        let rules = json!([
            { "id": "1111", "name": "orders-500" },
            { "id": "2222", "name": "orders-404" },
        ]);
        assert!(pick_named(&rules, "orders", "rule").is_err());
        assert_eq!(pick_named(&rules, "orders-500", "rule").unwrap(), "1111");
    }
}
