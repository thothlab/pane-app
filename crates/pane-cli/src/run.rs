//! Command implementations.

use std::io::Write;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use serde_json::{json, Value};

use crate::cli::*;
use crate::output::{exit, note, print_json, print_ndjson_line, Format};
use crate::session::Session;

/// Summary projection for capture rows.
///
/// Not the whole `CaptureDto`: `session_id`, `client_addr`, `scheme`,
/// `http_version` and the two body-id UUIDs cost roughly 100 characters per
/// row and are almost never what the caller is looking at. Body ids are
/// replaced by booleans because `captures body <capture-id>` resolves them
/// server-side anyway.
const SUMMARY_FIELDS: &[&str] = &[
    "id",
    "started_at",
    "method",
    "status",
    "server_host",
    "url_path",
    "duration_ms",
    "total_bytes",
    "state",
    "error_kind",
    "matched_rule_name",
];

fn summarize(cap: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for f in SUMMARY_FIELDS {
        if let Some(v) = cap.get(*f) {
            out.insert((*f).to_string(), v.clone());
        }
    }
    out.insert(
        "has_req_body".into(),
        json!(!cap.get("req_body_id").map(|v| v.is_null()).unwrap_or(true)),
    );
    out.insert(
        "has_res_body".into(),
        json!(!cap.get("res_body_id").map(|v| v.is_null()).unwrap_or(true)),
    );
    Value::Object(out)
}

fn project(cap: &Value, fields: &[String]) -> Value {
    let mut out = serde_json::Map::new();
    for f in fields {
        if let Some(v) = cap.get(f) {
            out.insert(f.clone(), v.clone());
        }
    }
    Value::Object(out)
}

fn short(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

fn human_capture_table(rows: &[Value]) {
    println!(
        "{:<9} {:<7} {:<6} {:<24} {:<32} {:>7} {:>9}  RULE",
        "SID", "METHOD", "STATUS", "HOST", "PATH", "MS", "BYTES"
    );
    for r in rows {
        let s = |k: &str| r.get(k).and_then(|v| v.as_str()).unwrap_or("");
        let n = |k: &str| {
            r.get(k)
                .and_then(|v| v.as_i64())
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".into())
        };
        let path = s("url_path");
        let path = if path.len() > 32 { &path[..32] } else { path };
        // The rule column is why `state` is worth surfacing in human output at
        // all: it is the difference between "a mock answered" and "this mock
        // answered".
        let rule = match (
            s("state"),
            r.get("matched_rule_name").and_then(|v| v.as_str()),
        ) {
            ("stubbed" | "patched", Some(name)) => name.to_string(),
            ("stubbed" | "patched", None) => "(mock)".into(),
            _ => String::new(),
        };
        println!(
            "{:<9} {:<7} {:<6} {:<24} {:<32} {:>7} {:>9}  {}",
            short(s("id")),
            s("method"),
            n("status"),
            s("server_host"),
            path,
            n("duration_ms"),
            n("total_bytes"),
            rule
        );
    }
}

/// Resolve a user-supplied selector to a UUID.
///
/// Accepts a full UUID, any unambiguous prefix (git-style), or a name
/// substring. Ambiguity is an error listing the candidates rather than a
/// silent pick — choosing for the user here would mean deleting or disabling
/// the wrong rule.
fn resolve(items: &[Value], selector: &str, name_field: &str, what: &str) -> Result<String> {
    resolve_by(items, selector, &[name_field], what).map(|(id, _)| id)
}

/// Selector resolution over several human-readable fields.
///
/// Devices need two of them: the captures DSL matches `device:` against display
/// name *or* serial (`filter_dsl::device_clause`), and a `--device` flag that
/// only understood serials would reject the very string the user just verified
/// their filter with. Returns the matched item alongside its id so a caller can
/// inspect the rest of the row.
fn resolve_by<'a>(
    items: &'a [Value],
    selector: &str,
    name_fields: &[&str],
    what: &str,
) -> Result<(String, &'a Value)> {
    let sel = selector.to_lowercase();
    let id_of = |v: &Value| {
        v.get("id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string()
    };
    let names_of = |v: &'a Value| -> Vec<&'a str> {
        name_fields
            .iter()
            .filter_map(|f| v.get(*f).and_then(|x| x.as_str()))
            .collect()
    };

    if let Some(hit) = items
        .iter()
        .find(|v| id_of(v).eq_ignore_ascii_case(selector))
    {
        return Ok((id_of(hit), hit));
    }
    let matches: Vec<&Value> = items
        .iter()
        .filter(|v| {
            id_of(v).to_lowercase().starts_with(&sel)
                || names_of(v).iter().any(|n| n.to_lowercase().contains(&sel))
        })
        .collect();

    match matches.len() {
        0 => Err(anyhow::Error::new(pane_core::api_err(
            pane_ipc::kinds::NOT_FOUND,
            format!("no {what} matches `{selector}`"),
        ))),
        1 => Ok((id_of(matches[0]), matches[0])),
        _ => {
            let names: Vec<String> = matches
                .iter()
                .map(|v| format!("{} {}", short(&id_of(v)), names_of(v).join(" ")))
                .collect();
            Err(anyhow!(
                "`{selector}` matches {} {what}s:\n  {}",
                matches.len(),
                names.join("\n  ")
            ))
        }
    }
}

fn as_array(v: Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}

fn require_yes(flag: bool, what: &str) -> Result<()> {
    if flag {
        return Ok(());
    }
    Err(anyhow!("{what} is destructive — pass --yes to confirm"))
}

// ─────────────────────────── entry point ───────────────────────────

pub async fn run(cli: Cli, format: Format) -> Result<i32> {
    let data_dir = crate::session::resolve_data_dir(cli.data_dir.clone())?;

    // These do not need a session at all.
    match &cli.command {
        Command::Schema => {
            print_json(&crate::schema::schema());
            return Ok(exit::OK);
        }
        Command::Install { dir } => return crate::install::install(dir.as_deref()),
        Command::Proxy(ProxyCmd::Run {
            host,
            port,
            no_proxy,
        }) => {
            return crate::headless::run_foreground(&data_dir, host, *port, *no_proxy).await;
        }
        Command::Mcp => return crate::mcp::serve(data_dir).await,
        _ => {}
    }

    let mut s = Session::open(data_dir.clone()).await?;
    dispatch(&mut s, cli, format, &data_dir).await
}

async fn dispatch(s: &mut Session, cli: Cli, format: Format, data_dir: &Path) -> Result<i32> {
    match cli.command {
        Command::Schema | Command::Install { .. } | Command::Mcp => unreachable!("handled above"),
        Command::Proxy(ProxyCmd::Run { .. }) => unreachable!("handled above"),

        Command::Doctor => doctor(s, format, data_dir).await,

        Command::Proxy(ProxyCmd::Start { host, port }) => {
            let v = s
                .call("proxy.start", json!({ "host": host, "port": port }))
                .await?;
            emit(&v, format, |v| {
                println!(
                    "proxy running on {}",
                    v.get("listen").and_then(|x| x.as_str()).unwrap_or("?")
                )
            });
            Ok(exit::OK)
        }
        Command::Proxy(ProxyCmd::Stop) => {
            let v = s.call("proxy.stop", Value::Null).await?;
            emit(&v, format, |v| {
                println!(
                    "proxy stopped; cleared {} device(s)",
                    v.get("cleared_devices")
                        .and_then(|x| x.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0)
                )
            });
            Ok(exit::OK)
        }
        Command::Proxy(ProxyCmd::Status) => {
            let v = s.call("proxy.status", Value::Null).await?;
            emit(&v, format, |v| {
                let running = v.get("running").and_then(|x| x.as_bool()).unwrap_or(false);
                println!(
                    "{:<10} {:<20} {} captures",
                    if running { "running" } else { "stopped" },
                    v.get("listen").and_then(|x| x.as_str()).unwrap_or("-"),
                    v.get("captures_count")
                        .and_then(|x| x.as_u64())
                        .unwrap_or(0)
                )
            });
            Ok(exit::OK)
        }

        Command::Tail(a) | Command::Captures(CapturesCmd::Tail(a)) => tail(s, a).await,

        Command::Captures(c) => captures(s, c, format).await,
        Command::Rules(c) => rules(s, c, format).await,
        Command::Collections(c) => collections(s, c, format).await,
        Command::Devices(c) => devices(s, c, format).await,
        Command::Logcat(c) => logcat(s, c, format).await,
        Command::Ca(c) => ca(s, c, format).await,
    }
}

fn emit(v: &Value, format: Format, human: impl Fn(&Value)) {
    match format {
        Format::Json => print_json(v),
        Format::Human => human(v),
    }
}

// ─────────────────────────── doctor ───────────────────────────

async fn doctor(s: &mut Session, format: Format, data_dir: &Path) -> Result<i32> {
    let proxy = s
        .call("proxy.status", Value::Null)
        .await
        .unwrap_or(Value::Null);
    let tooling = s
        .call("devices.tooling_status", Value::Null)
        .await
        .unwrap_or(Value::Null);
    let paired = s
        .call("devices.list", Value::Null)
        .await
        .unwrap_or(json!([]));
    let attached = s
        .call("devices.attached", Value::Null)
        .await
        .unwrap_or(json!([]));
    let ca = s
        .call("ca.current", Value::Null)
        .await
        .unwrap_or(Value::Null);

    let report = json!({
        "attached_to_instance": s.is_attached(),
        "data_dir": data_dir,
        "proxy": proxy,
        "android_tooling": tooling,
        "devices_paired": paired,
        "devices_attached": attached,
        "ca": ca,
    });

    if format == Format::Json {
        print_json(&report);
        return Ok(exit::OK);
    }

    let running = report["proxy"]["running"].as_bool().unwrap_or(false);
    println!(
        "{:<11}{}",
        "instance",
        if s.is_attached() {
            "attached to a running Pane"
        } else {
            "none running — reading the data directory directly"
        }
    );
    println!(
        "{:<11}{:<10}{:<20}{} captures",
        "proxy",
        if running { "running" } else { "stopped" },
        report["proxy"]["listen"].as_str().unwrap_or("-"),
        report["proxy"]["captures_count"].as_u64().unwrap_or(0)
    );
    println!(
        "{:<11}{}",
        "adb",
        if report["android_tooling"]["ok"].as_bool().unwrap_or(false) {
            report["android_tooling"]["adb_path"]
                .as_str()
                .unwrap_or("ok")
                .to_string()
        } else {
            format!(
                "MISSING — {}",
                report["android_tooling"]["error"]
                    .as_str()
                    .unwrap_or("adb not found")
            )
        }
    );
    println!(
        "{:<11}{}",
        "ca",
        report["ca"]["sha256_fp"]
            .as_str()
            .map(|fp| format!("{}…", &fp[..fp.len().min(16)]))
            .unwrap_or_else(|| "none".into())
    );
    println!(
        "{:<11}{} paired · {} attached",
        "devices",
        report["devices_paired"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0),
        report["devices_attached"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0)
    );
    for d in report["devices_paired"].as_array().unwrap_or(&vec![]) {
        println!(
            "  {:<14}{:<18}{:<10}{}",
            d["serial"].as_str().unwrap_or(""),
            d["display_name"].as_str().unwrap_or(""),
            d["platform"].as_str().unwrap_or(""),
            d["state"].as_str().unwrap_or("")
        );
    }
    Ok(exit::OK)
}

// ─────────────────────────── captures ───────────────────────────

async fn captures(s: &mut Session, cmd: CapturesCmd, format: Format) -> Result<i32> {
    match cmd {
        CapturesCmd::Tail(_) => unreachable!("handled by dispatch"),

        CapturesCmd::List {
            filter,
            limit,
            fields,
            full,
        } => {
            let v = s
                .call(
                    "captures.list",
                    json!({ "filter": filter, "limit": limit, "before": null }),
                )
                .await?;
            let rows = as_array(v);
            let shaped: Vec<Value> = match (&fields, full) {
                (Some(f), _) => {
                    let names: Vec<String> = f
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    rows.iter().map(|r| project(r, &names)).collect()
                }
                (None, true) => rows.clone(),
                (None, false) => rows.iter().map(summarize).collect(),
            };
            match format {
                Format::Json => print_json(&Value::Array(shaped)),
                Format::Human => human_capture_table(&rows),
            }
            Ok(exit::OK)
        }

        CapturesCmd::Count { filter } => {
            // Count by listing at the storage cap: there is no dedicated
            // filtered-count op, and this stays honest about the ceiling
            // rather than reporting a number the query could not have reached.
            let v = s
                .call(
                    "captures.list",
                    json!({ "filter": filter, "limit": 2000, "before": null }),
                )
                .await?;
            println!("{}", as_array(v).len());
            Ok(exit::OK)
        }

        CapturesCmd::Get { id } => {
            let id = resolve_capture(s, &id).await?;
            let v = s.call("captures.get", json!(id)).await?;
            emit(&v, format, print_json);
            Ok(exit::OK)
        }

        CapturesCmd::Body {
            id,
            res,
            req,
            max_bytes,
            out,
            base64: want_b64,
        } => {
            let id = resolve_capture(s, &id).await?;
            let cap = s.call("captures.get", json!(id)).await?;
            let side_field = if req { "req_body_id" } else { "res_body_id" };
            let _ = res;
            let body_id = cap
                .get(side_field)
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    anyhow::Error::new(pane_core::api_err(
                        pane_ipc::kinds::NOT_FOUND,
                        format!(
                            "this capture has no {} body",
                            if req { "request" } else { "response" }
                        ),
                    ))
                })?
                .to_string();

            // Writing to a file costs no context, so never truncate there.
            let effective_max = if out.is_some() || max_bytes == 0 {
                Value::Null
            } else {
                json!(max_bytes)
            };
            let body = s
                .call(
                    "captures.body",
                    json!({ "body_id": body_id, "max_bytes": effective_max }),
                )
                .await?;

            let b64 = body
                .get("bytes_base64")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if want_b64 {
                println!("{b64}");
                return Ok(exit::OK);
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .context("decoding body")?;

            if let Some(path) = out {
                std::fs::write(&path, &bytes)
                    .with_context(|| format!("writing {}", path.display()))?;
                note(format!("{} bytes → {}", bytes.len(), path.display()));
            } else {
                std::io::stdout().write_all(&bytes)?;
                if !bytes.ends_with(b"\n") {
                    println!();
                }
                let total = body.get("total_size").and_then(|v| v.as_u64()).unwrap_or(0);
                if body
                    .get("truncated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    note(format!(
                        "body truncated: {} of {total} bytes (use --max-bytes 0, or --out FILE)",
                        bytes.len()
                    ));
                }
            }
            Ok(exit::OK)
        }

        CapturesCmd::Export { id, format: f, out } => {
            let id = resolve_capture(s, &id).await?;
            let wire = if f == "har" { "har_single" } else { "curl" };
            let v = s
                .call("captures.export", json!({ "id": id, "format": wire }))
                .await?;
            let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("");
            match out {
                Some(p) => {
                    std::fs::write(&p, text)?;
                    note(format!("→ {}", p.display()));
                }
                None => println!("{text}"),
            }
            Ok(exit::OK)
        }

        CapturesCmd::Clear { r#yes } => {
            require_yes(r#yes, "clearing every capture")?;
            let v = s.call("captures.clear", json!({})).await?;
            emit(&v, format, |v| {
                println!(
                    "deleted {}",
                    v.get("deleted").and_then(|x| x.as_u64()).unwrap_or(0)
                )
            });
            Ok(exit::OK)
        }
    }
}

/// Capture ids are opaque, so a prefix is resolved by listing recent rows.
async fn resolve_capture(s: &mut Session, selector: &str) -> Result<String> {
    if uuid::Uuid::parse_str(selector).is_ok() {
        return Ok(selector.to_string());
    }
    let rows = as_array(
        s.call(
            "captures.list",
            json!({ "filter": null, "limit": 2000, "before": null }),
        )
        .await?,
    );
    resolve(&rows, selector, "url_path", "capture")
}

// ─────────────────────────── tail ───────────────────────────

async fn tail(s: &mut Session, args: TailArgs) -> Result<i32> {
    let Session::Attached(client) = s else {
        return Err(anyhow!(
            "`tail` needs a running Pane instance — start one with `pane proxy run`, \
             or open the Pane app"
        ));
    };

    // Validate the filter before announcing readiness, so a typo fails now
    // rather than after the caller has already launched the app under test.
    if let Some(f) = args.filter.as_deref() {
        pane_storage::validate_filter(f).map_err(|e| {
            anyhow::Error::new(pane_core::api_err(
                pane_ipc::kinds::FILTER_PARSE,
                e.to_string(),
            ))
        })?;
    }

    // `ready` first, always. A scripted run reads this line and only then
    // triggers the app, which is what makes the loop race-free — without it
    // every such script needs a sleep and becomes flaky.
    print_ndjson_line(&json!({
        "event": "ready",
        "filter": args.filter,
        "count": args.count,
        "timeout": args.timeout,
    }));

    let want = args.count.unwrap_or(usize::MAX);
    let mut seen = 0usize;
    let started = std::time::Instant::now();

    let sub = pane_control::SubscribeArgs {
        topics: vec!["capture.completed".into(), "capture.error".into()],
        filter: args.filter.clone(),
        enrich: "summary".into(),
    };

    let stream = client.subscribe(sub, |ev| {
        if ev.topic == "capture.error" {
            print_ndjson_line(&json!({ "event": "error", "capture": ev.payload }));
            return true;
        }
        let mut line = summarize(&ev.payload);
        if let Value::Object(ref mut m) = line {
            m.insert("event".into(), json!("capture"));
        }
        print_ndjson_line(&line);
        seen += 1;
        seen < want
    });

    let outcome = match args.timeout {
        Some(secs) => {
            match tokio::time::timeout(std::time::Duration::from_secs(secs), stream).await {
                Ok(r) => {
                    r?;
                    "count"
                }
                Err(_) => "timeout",
            }
        }
        None => {
            stream.await?;
            "count"
        }
    };

    print_ndjson_line(&json!({
        "event": "end",
        "reason": outcome,
        "captures": seen,
        "elapsed_ms": started.elapsed().as_millis() as u64,
    }));

    // The count is the assertion; the timeout is the deadline. Falling short
    // of an explicit --count is a failed assertion, and scripts branch on it.
    if outcome == "timeout" && args.count.is_some() && seen < want {
        return Ok(exit::TIMEOUT);
    }
    Ok(exit::OK)
}

// ─────────────────────────── rules ───────────────────────────

async fn rules(s: &mut Session, cmd: RulesCmd, format: Format) -> Result<i32> {
    match cmd {
        RulesCmd::Ls { device } => {
            let scope = match device.as_deref() {
                Some(sel) => Some(resolve_device(s, sel).await?),
                None => None,
            };
            let v = s.call("rules.list", Value::Null).await?;
            match format {
                Format::Json => print_json(&v),
                Format::Human => {
                    // With a device named, STATE answers "is this live on that
                    // phone" rather than reporting the raw flag — the rows are
                    // all still listed, because "why isn't my mock firing" has
                    // to be answerable without guessing which rules were hidden.
                    println!(
                        "{:<9} {:<7} {:<10} {:<28} {:<7} MATCH",
                        "ID",
                        if scope.is_some() { "LIVE" } else { "STATE" },
                        "SCOPE",
                        "NAME",
                        "STATUS"
                    );
                    for r in as_array(v) {
                        let state = match scope.as_deref() {
                            Some(d) => live_on(&r, d),
                            None => r["enabled"].as_bool().unwrap_or(false),
                        };
                        println!(
                            "{:<9} {:<7} {:<10} {:<28} {:<7} {} {}{}",
                            short(r["id"].as_str().unwrap_or("")),
                            if state { "on" } else { "off" },
                            scope_label(&r),
                            r["name"].as_str().unwrap_or(""),
                            r["res_status"].as_u64().unwrap_or(0),
                            r["match_method"].as_str().unwrap_or("*"),
                            r["match_host_glob"].as_str().unwrap_or("*"),
                            r["match_path_glob"].as_str().unwrap_or(""),
                        );
                    }
                }
            }
            Ok(exit::OK)
        }
        RulesCmd::Get { selector } => {
            let id = resolve_rule(s, &selector).await?;
            let v = s.call("rules.get", json!(id)).await?;
            print_json(&v);
            Ok(exit::OK)
        }
        RulesCmd::Enable {
            selector,
            all,
            collection,
            ungrouped,
            device,
        } => set_rules_enabled(s, selector, all, collection, ungrouped, device, true).await,
        RulesCmd::Disable {
            selector,
            all,
            collection,
            ungrouped,
            device,
        } => set_rules_enabled(s, selector, all, collection, ungrouped, device, false).await,
        RulesCmd::Rm { selector, r#yes } => {
            require_yes(r#yes, "deleting a rule")?;
            let id = resolve_rule(s, &selector).await?;
            s.call("rules.delete", json!(id)).await?;
            note(format!("deleted {}", short(&id)));
            Ok(exit::OK)
        }
        RulesCmd::Mock {
            host,
            path,
            method,
            status,
            body,
            body_file,
            mime,
            delay_ms,
            name,
            disabled,
            device,
        } => {
            let body_b64 = read_body(body, body_file)?;
            // Scoped at creation rather than created-then-narrowed: narrowing a
            // rule that is already live everywhere has to expand the wildcard
            // for the whole library, and a brand-new rule should not drag the
            // rest of it into device scope.
            let scope = match device.as_deref() {
                Some(sel) => Some(resolve_device(s, sel).await?),
                None => None,
            };
            let args = json!({
                "id": null,
                "name": name.unwrap_or_else(|| format!("{host}{}", path.clone().unwrap_or_default())),
                "enabled": !disabled,
                "priority": 0,
                "collection_id": null,
                "mode": "stub",
                "patches": [],
                "match_host_glob": host,
                "match_method": method,
                "match_path_glob": path,
                "match_params": [],
                "match_req_body": null,
                "match_conditions": [],
                "res_status": status,
                "res_headers": [{ "name": "content-type", "value": mime }],
                "res_body_id": null,
                "res_body_base64": body_b64,
                "res_body_mime": mime,
                "res_delay_ms": delay_ms,
                "enabled_scope": scope.as_ref().map(|_| "set"),
                "devices": scope.as_ref().map(|d| vec![d.clone()]),
            });
            let v = s.call("rules.upsert", args).await?;
            emit(&v, format, |v| {
                println!(
                    "rule {}  \"{}\"  {}  {} {}",
                    short(v["id"].as_str().unwrap_or("")),
                    v["name"].as_str().unwrap_or(""),
                    if v["enabled"].as_bool().unwrap_or(false) {
                        "enabled"
                    } else {
                        "disabled"
                    },
                    v["res_status"].as_u64().unwrap_or(0),
                    v["match_host_glob"].as_str().unwrap_or("")
                )
            });
            Ok(exit::OK)
        }
        RulesCmd::FromCapture {
            id,
            status,
            body,
            body_file,
            name,
        } => {
            let cap_id = resolve_capture(s, &id).await?;
            let cap = s.call("captures.get", json!(cap_id)).await?;
            let host = cap["server_host"].as_str().unwrap_or_default().to_string();
            let full_path = cap["url_path"].as_str().unwrap_or("/");
            // Drop the query string: matching on the path shape is what makes
            // the derived rule reusable across runs.
            let path = full_path.split('?').next().unwrap_or(full_path).to_string();
            let body_b64 = read_body(body, body_file)?;
            // With no explicit body, reuse the captured response body as-is —
            // res_body_id already points at it, so nothing is copied.
            let reuse_body_id = if body_b64.is_none() {
                cap.get("res_body_id").cloned().unwrap_or(Value::Null)
            } else {
                Value::Null
            };

            let args = json!({
                "id": null,
                "name": name.unwrap_or_else(|| format!("{host}{path}")),
                "enabled": true,
                "priority": 0,
                "collection_id": null,
                "mode": "stub",
                "patches": [],
                "match_host_glob": host,
                "match_method": cap["method"].as_str(),
                "match_path_glob": path,
                "match_params": [],
                "match_req_body": null,
                "match_conditions": [],
                "res_status": status.unwrap_or_else(|| cap["status"].as_u64().unwrap_or(200) as u16),
                "res_headers": [{ "name": "content-type", "value": "application/json" }],
                "res_body_id": reuse_body_id,
                "res_body_base64": body_b64,
                "res_body_mime": "application/json",
                "res_delay_ms": 0,
            });
            let v = s.call("rules.upsert", args).await?;
            emit(&v, format, |v| {
                println!(
                    "rule {}  \"{}\"  {} {}{}",
                    short(v["id"].as_str().unwrap_or("")),
                    v["name"].as_str().unwrap_or(""),
                    v["res_status"].as_u64().unwrap_or(0),
                    v["match_host_glob"].as_str().unwrap_or(""),
                    v["match_path_glob"].as_str().unwrap_or("")
                )
            });
            Ok(exit::OK)
        }
        RulesCmd::Import { file, dry_run } => {
            crate::portfile::import(s, &file, dry_run, format).await
        }
        RulesCmd::Export { out } => crate::portfile::export(s, out.as_deref()).await,
    }
}

/// One rule by selector, or a whole scope via `--all` / `--collection` /
/// `--ungrouped`. Exactly one of the four must be given; clap enforces that
/// they don't combine, this catches the case where none were.
#[allow(clippy::too_many_arguments)]
async fn set_rules_enabled(
    s: &mut Session,
    selector: Option<String>,
    all: bool,
    collection: Option<String>,
    ungrouped: bool,
    device: Option<String>,
    enabled: bool,
) -> Result<i32> {
    let verb = if enabled { "enabled" } else { "disabled" };
    let dev = match device.as_deref() {
        Some(sel) => Some(resolve_device(s, sel).await?),
        None => None,
    };

    let scope = if all {
        json!({ "kind": "all" })
    } else if ungrouped {
        json!({ "kind": "ungrouped" })
    } else if let Some(sel) = collection {
        json!({ "kind": "collection", "id": resolve_collection(s, &sel).await? })
    } else if let Some(sel) = selector {
        // Single-rule path keeps the narrower op: it reports "not found" for a
        // bad selector, where a bulk call would cheerfully match zero rules.
        let id = resolve_rule(s, &sel).await?;
        s.call(
            "rules.set_enabled",
            json!({ "id": id, "enabled": enabled, "device": dev }),
        )
        .await?;
        note(format!("{verb} {}{}", short(&id), for_device(&dev)));
        return Ok(exit::OK);
    } else {
        return Err(anyhow!(
            "give a rule selector, or one of --all / --collection <sel> / --ungrouped"
        ));
    };

    let v = s
        .call(
            "rules.set_enabled_bulk",
            json!({ "enabled": enabled, "scope": scope, "device": dev }),
        )
        .await?;
    let matched = v.get("matched").and_then(|x| x.as_u64()).unwrap_or(0);
    let changed = v.get("changed").and_then(|x| x.as_u64()).unwrap_or(0);
    note(format!(
        "{verb} {changed} of {matched} rule{}{}",
        if matched == 1 { "" } else { "s" },
        for_device(&dev)
    ));
    report_materialized(&v);
    Ok(exit::OK)
}

fn for_device(dev: &Option<String>) -> String {
    match dev {
        Some(d) => format!(" for {}", short(d)),
        None => String::new(),
    }
}

/// Say when rules stopped being on-for-every-device.
///
/// Switching a rule off for one phone has to pin it to the phones it stays on
/// for, and that quietly decides what a device paired *later* will see: nothing.
/// It is reversible with a plain `pane rules enable --all`, but only if the user
/// knows it happened.
fn report_materialized(v: &Value) {
    let n = v.get("materialized").and_then(|x| x.as_u64()).unwrap_or(0);
    if n > 0 {
        note(format!(
            "{n} rule(s) are now pinned to named devices — a device paired from \
             here on will not get them (undo with `pane rules enable --all`)"
        ));
    }
}

/// Resolve a `--device` selector to the scope id the engine will see.
///
/// `__host__` passes straight through: this Mac's own traffic is a scope like
/// any other, and there is no `device` row to look it up in.
async fn resolve_device(s: &mut Session, selector: &str) -> Result<String> {
    if selector == pane_storage::SCOPE_HOST {
        return Ok(selector.to_string());
    }
    let list = as_array(s.call("devices.list", Value::Null).await?);
    let (id, dev) = resolve_by(&list, selector, &["display_name", "serial"], "device")?;
    // An iOS device is forwarded 8888 → 8888 and never gets a port of its own,
    // so its traffic is never stamped with this id. Scoping a rule to it would
    // be accepted and then silently do nothing at runtime — an hour of "why
    // isn't my mock firing" for a flag that could just say so.
    if dev.get("platform").and_then(|p| p.as_str()) == Some("ios") {
        return Err(anyhow!(
            "`{selector}` is an iOS device: its traffic shares the host proxy port and \
             is not attributed per device, so a rule cannot be scoped to it. Leave the \
             rule global, or scope it to `__host__`."
        ));
    }
    Ok(id)
}

/// Is this rule live for one scope? Mirrors the engine's SQL predicate.
fn live_on(rule: &Value, scope: &str) -> bool {
    if !rule["enabled"].as_bool().unwrap_or(false) {
        return false;
    }
    if rule["enabled_scope"].as_str().unwrap_or("all") == "all" {
        return true;
    }
    rule["devices"]
        .as_array()
        .map(|ds| ds.iter().any(|d| d.as_str() == Some(scope)))
        .unwrap_or(false)
}

/// `all`, or how many real devices a pinned rule names.
///
/// The two sentinels — this Mac and unattributed traffic — are in the stored set
/// but deliberately not in the count: "4 dev" on a desk with two phones reads as
/// a bug. `pane rules get` prints the full set for anyone who needs it.
fn scope_label(rule: &Value) -> String {
    if rule["enabled_scope"].as_str().unwrap_or("all") == "all" {
        return "all".to_string();
    }
    let n = rule["devices"]
        .as_array()
        .map(|d| {
            d.iter()
                .filter(|v| !matches!(v.as_str(), Some("__host__") | Some("__none__")))
                .count()
        })
        .unwrap_or(0);
    match n {
        0 => "pinned".to_string(),
        n => format!("{n} dev"),
    }
}

async fn resolve_rule(s: &mut Session, selector: &str) -> Result<String> {
    let rules = as_array(s.call("rules.list", Value::Null).await?);
    resolve(&rules, selector, "name", "rule")
}

fn read_body(inline: Option<String>, file: Option<std::path::PathBuf>) -> Result<Option<Value>> {
    let bytes = match (inline, file) {
        (Some(s), _) => s.into_bytes(),
        (None, Some(p)) => std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?,
        (None, None) => return Ok(None),
    };
    Ok(Some(json!(
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )))
}

// ─────────────────────────── collections ───────────────────────────

async fn collections(s: &mut Session, cmd: CollectionsCmd, format: Format) -> Result<i32> {
    match cmd {
        CollectionsCmd::Ls => {
            let v = s.call("collections.list", Value::Null).await?;
            match format {
                Format::Json => print_json(&v),
                Format::Human => {
                    println!("{:<9} {:<7} {:>6}  NAME", "ID", "STATE", "RULES");
                    for c in as_array(v) {
                        println!(
                            "{:<9} {:<7} {:>6}  {}",
                            short(c["id"].as_str().unwrap_or("")),
                            if c["enabled"].as_bool().unwrap_or(false) {
                                "on"
                            } else {
                                "off"
                            },
                            c["rule_count"].as_u64().unwrap_or(0),
                            c["name"].as_str().unwrap_or("")
                        );
                    }
                }
            }
            Ok(exit::OK)
        }
        CollectionsCmd::Enable { selector, device } => {
            set_collection(s, &selector, device, true).await
        }
        CollectionsCmd::Disable { selector, device } => {
            set_collection(s, &selector, device, false).await
        }
        CollectionsCmd::Only { selector, device } => {
            // "Only this scenario" = clear every checkbox, then tick this
            // collection's. Expressed in `rule.enabled`, the one flag the
            // engine reads and the user can see in the list — there is no
            // separate collection switch to fall out of sync with it.
            let all = as_array(s.call("collections.list", Value::Null).await?);
            let keep = resolve(&all, &selector, "name", "collection")?;
            // With a device named this is "switch THIS phone to that scenario":
            // every other device keeps whatever it was running, which is the
            // whole point of running two scenarios side by side.
            let dev = match device.as_deref() {
                Some(sel) => Some(resolve_device(s, sel).await?),
                None => None,
            };

            let off = s
                .call(
                    "rules.set_enabled_bulk",
                    json!({ "enabled": false, "scope": { "kind": "all" }, "device": dev }),
                )
                .await?;
            let v = s
                .call(
                    "rules.set_enabled_bulk",
                    json!({
                        "enabled": true,
                        "scope": { "kind": "collection", "id": keep },
                        "device": dev,
                    }),
                )
                .await?;
            let on = v.get("matched").and_then(|x| x.as_u64()).unwrap_or(0);

            note(format!(
                "only `{}` is live{} — {on} rule(s) enabled, everything else off",
                all.iter()
                    .find(|c| c["id"].as_str() == Some(keep.as_str()))
                    .and_then(|c| c["name"].as_str())
                    .unwrap_or(&keep),
                for_device(&dev)
            ));
            report_materialized(&off);
            Ok(exit::OK)
        }
        CollectionsCmd::Rm {
            selector,
            with_rules,
            r#yes,
        } => {
            require_yes(
                r#yes,
                if with_rules {
                    "deleting a collection and its rules"
                } else {
                    "deleting a collection"
                },
            )?;
            let id = resolve_collection(s, &selector).await?;

            // Order matters: drop the rules first, because `collections.delete`
            // detaches whatever is still attached. Doing it the other way round
            // would orphan them to Ungrouped and then delete nothing.
            let removed = if with_rules {
                let r = s
                    .call(
                        "rules.set_enabled_bulk",
                        json!({ "enabled": false, "scope": { "kind": "collection", "id": id } }),
                    )
                    .await?;
                let n = r.get("matched").and_then(|x| x.as_u64()).unwrap_or(0);
                for rule in as_array(s.call("rules.list", Value::Null).await?)
                    .iter()
                    .filter(|r| r["collection_id"].as_str() == Some(id.as_str()))
                {
                    s.call("rules.delete", json!(rule["id"].as_str().unwrap_or("")))
                        .await?;
                }
                n
            } else {
                0
            };

            s.call("collections.delete", json!(id)).await?;
            note(if with_rules {
                format!("deleted {} and {removed} rule(s)", short(&id))
            } else {
                format!("deleted {} — its rules moved to Ungrouped", short(&id))
            });
            Ok(exit::OK)
        }
    }
}

async fn resolve_collection(s: &mut Session, selector: &str) -> Result<String> {
    let all = as_array(s.call("collections.list", Value::Null).await?);
    resolve(&all, selector, "name", "collection")
}

/// Tick or untick every rule in a collection.
///
/// Deliberately not a write to `rule_collection.enabled`: that column exists
/// but nothing reads it when deciding what serves traffic, so writing it would
/// report success and change nothing.
async fn set_collection(
    s: &mut Session,
    selector: &str,
    device: Option<String>,
    enabled: bool,
) -> Result<i32> {
    let id = resolve_collection(s, selector).await?;
    let dev = match device.as_deref() {
        Some(sel) => Some(resolve_device(s, sel).await?),
        None => None,
    };
    let v = s
        .call(
            "rules.set_enabled_bulk",
            json!({
                "enabled": enabled,
                "scope": { "kind": "collection", "id": id },
                "device": dev,
            }),
        )
        .await?;
    let matched = v.get("matched").and_then(|x| x.as_u64()).unwrap_or(0);
    let changed = v.get("changed").and_then(|x| x.as_u64()).unwrap_or(0);
    note(format!(
        "{} {changed} of {matched} rule(s) in {}{}",
        if enabled { "enabled" } else { "disabled" },
        short(&id),
        for_device(&dev)
    ));
    report_materialized(&v);
    Ok(exit::OK)
}

// ─────────────────────────── devices ───────────────────────────

async fn devices(s: &mut Session, cmd: DevicesCmd, format: Format) -> Result<i32> {
    match cmd {
        DevicesCmd::Ls => {
            let v = s.call("devices.list", Value::Null).await?;
            match format {
                Format::Json => print_json(&v),
                Format::Human => {
                    for d in as_array(v) {
                        println!(
                            "{:<9} {:<14} {:<18} {:<9} {}",
                            short(d["id"].as_str().unwrap_or("")),
                            d["serial"].as_str().unwrap_or(""),
                            d["display_name"].as_str().unwrap_or(""),
                            d["platform"].as_str().unwrap_or(""),
                            d["state"].as_str().unwrap_or("")
                        );
                    }
                }
            }
            Ok(exit::OK)
        }
        DevicesCmd::Attached => {
            let v = s.call("devices.attached", Value::Null).await?;
            match format {
                Format::Json => print_json(&v),
                Format::Human => {
                    for d in as_array(v) {
                        println!(
                            "{:<14} {:<9} {}",
                            d["serial"].as_str().unwrap_or(""),
                            d["platform"].as_str().unwrap_or(""),
                            d["name"].as_str().unwrap_or("")
                        );
                    }
                }
            }
            Ok(exit::OK)
        }
        DevicesCmd::Add { serial, platform } => {
            let platform = match platform {
                Some(p) => p,
                None => {
                    let attached = as_array(s.call("devices.attached", Value::Null).await?);
                    attached
                        .iter()
                        .find(|d| d["serial"].as_str() == Some(serial.as_str()))
                        .and_then(|d| d["platform"].as_str())
                        .map(String::from)
                        .ok_or_else(|| {
                            anyhow::Error::new(pane_core::api_err(
                                pane_ipc::kinds::NOT_FOUND,
                                format!(
                                    "`{serial}` is not attached; run `pane devices attached`, \
                                     or pass --platform"
                                ),
                            ))
                        })?
                }
            };
            let op = if platform == "ios" {
                "devices.add_ios"
            } else {
                "devices.add_android"
            };
            let v = s.call(op, json!({ "serial": serial })).await?;
            emit(&v, format, |v| {
                println!(
                    "paired {} ({})",
                    v["display_name"].as_str().unwrap_or(""),
                    v["state"].as_str().unwrap_or("")
                )
            });
            Ok(exit::OK)
        }
        DevicesCmd::Rm { selector, r#yes } => {
            require_yes(r#yes, "unpairing a device")?;
            let list = as_array(s.call("devices.list", Value::Null).await?);
            // Same two fields the captures `device:` filter matches on, so a
            // selector that worked there works here. Not `resolve_device`: that
            // one refuses iOS, and unpairing an iOS device has to keep working.
            let (id, _) = resolve_by(&list, &selector, &["display_name", "serial"], "device")?;
            let v = s.call("devices.remove", json!(id)).await?;
            emit(&v, format, |_| note("device removed"));
            Ok(exit::OK)
        }
    }
}

// ─────────────────────────── logcat ───────────────────────────

async fn logcat(s: &mut Session, cmd: LogcatCmd, format: Format) -> Result<i32> {
    match cmd {
        LogcatCmd::Attach { serial } => {
            let v = s.call("logcat.start", json!({ "serial": serial })).await?;
            emit(&v, format, |v| {
                println!(
                    "logcat {}",
                    if v["reused"].as_bool().unwrap_or(false) {
                        "already running"
                    } else {
                        "started"
                    }
                )
            });
            Ok(exit::OK)
        }
        LogcatCmd::Detach { serial } => {
            s.call("logcat.stop", json!({ "serial": serial })).await?;
            note("logcat stopped");
            Ok(exit::OK)
        }
        LogcatCmd::Query {
            serial,
            filter,
            limit,
        } => {
            // `app:` terms must be resolved to PIDs before the query — the
            // backend takes numeric pids only. Resolving to nothing has to
            // become a pid that matches nothing, or the filter silently
            // widens to the whole firehose.
            let (include, exclude) =
                crate::logcat_app::resolve_app_pids(s, &serial, filter.as_deref()).await?;
            let v = s
                .call(
                    "logcat.query",
                    json!({
                        "serial": serial,
                        "filter": filter,
                        "include_pids": include,
                        "exclude_pids": exclude,
                        "limit": limit,
                    }),
                )
                .await?;
            match format {
                Format::Json => print_json(&v),
                Format::Human => {
                    for r in as_array(v) {
                        println!(
                            "{}  {:>6}  {:<2} {:<22} {}",
                            r["timestamp"].as_str().unwrap_or(""),
                            r["pid"].as_u64().unwrap_or(0),
                            r["level"]
                                .as_str()
                                .unwrap_or("")
                                .chars()
                                .next()
                                .unwrap_or(' ')
                                .to_uppercase(),
                            r["tag"].as_str().unwrap_or(""),
                            r["message"].as_str().unwrap_or("")
                        );
                    }
                }
            }
            Ok(exit::OK)
        }
        LogcatCmd::Pids { serial } => {
            let v = s
                .call("logcat.pid_names", json!({ "serial": serial }))
                .await?;
            print_json(&v);
            Ok(exit::OK)
        }
        LogcatCmd::Clear { serial, r#yes } => {
            require_yes(r#yes, "clearing logcat rows")?;
            let v = s.call("logcat.clear", json!({ "serial": serial })).await?;
            emit(&v, format, |v| {
                println!("deleted {}", v["deleted"].as_u64().unwrap_or(0))
            });
            Ok(exit::OK)
        }
    }
}

// ─────────────────────────── ca ───────────────────────────

async fn ca(s: &mut Session, cmd: CaCmd, format: Format) -> Result<i32> {
    match cmd {
        CaCmd::Show => {
            let v = s.call("ca.current", Value::Null).await?;
            emit(&v, format, |v| {
                println!(
                    "{}\n  sha256 {}\n  valid  {} → {}",
                    v["subject"].as_str().unwrap_or(""),
                    v["sha256_fp"].as_str().unwrap_or(""),
                    v["valid_from"].as_str().unwrap_or(""),
                    v["valid_to"].as_str().unwrap_or("")
                )
            });
            Ok(exit::OK)
        }
        CaCmd::Export { format: f, out } => {
            let v = s.call("ca.export", json!({ "format": f })).await?;
            let b64 = v["data_base64"].as_str().unwrap_or("");
            let bytes = base64::engine::general_purpose::STANDARD.decode(b64)?;
            match out {
                Some(p) => {
                    std::fs::write(&p, &bytes)?;
                    note(format!("{} bytes → {}", bytes.len(), p.display()));
                }
                None => std::io::stdout().write_all(&bytes)?,
            }
            Ok(exit::OK)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str, name: &str) -> Value {
        json!({ "id": id, "name": name })
    }

    #[test]
    fn selector_matches_full_id_prefix_and_name() {
        let items = vec![
            rule("3b8e21f4-0000-0000-0000-000000000000", "orders-500"),
            rule("9c04e1d2-0000-0000-0000-000000000000", "cart-empty"),
        ];
        assert_eq!(
            resolve(
                &items,
                "3b8e21f4-0000-0000-0000-000000000000",
                "name",
                "rule"
            )
            .unwrap(),
            "3b8e21f4-0000-0000-0000-000000000000"
        );
        assert_eq!(
            resolve(&items, "3b8e", "name", "rule").unwrap(),
            "3b8e21f4-0000-0000-0000-000000000000"
        );
        assert_eq!(
            resolve(&items, "cart", "name", "rule").unwrap(),
            "9c04e1d2-0000-0000-0000-000000000000"
        );
    }

    /// Picking one silently would disable or delete the wrong rule.
    #[test]
    fn ambiguous_selector_is_an_error_listing_candidates() {
        let items = vec![
            rule("1111aaaa-0000-0000-0000-000000000000", "orders-500"),
            rule("2222bbbb-0000-0000-0000-000000000000", "orders-404"),
        ];
        let err = resolve(&items, "orders", "name", "rule").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("matches 2"), "{msg}");
        assert!(
            msg.contains("orders-500") && msg.contains("orders-404"),
            "{msg}"
        );
    }

    #[test]
    fn missing_selector_reports_not_found_kind() {
        let items = vec![rule("1111aaaa-0000-0000-0000-000000000000", "orders-500")];
        let err = resolve(&items, "nope", "name", "rule").unwrap_err();
        let api = err.downcast_ref::<pane_ipc::ApiError>().expect("ApiError");
        assert_eq!(api.kind, pane_ipc::kinds::NOT_FOUND);
    }

    #[test]
    fn summary_drops_body_uuids_for_booleans() {
        let cap = json!({
            "id": "x", "method": "GET", "status": 200, "server_host": "h",
            "url_path": "/", "duration_ms": 5, "total_bytes": 9, "state": "completed",
            "session_id": "should-be-dropped", "client_addr": "should-be-dropped",
            "req_body_id": null, "res_body_id": "abc"
        });
        let s = summarize(&cap);
        assert_eq!(s["has_req_body"], json!(false));
        assert_eq!(s["has_res_body"], json!(true));
        assert!(s.get("res_body_id").is_none());
        assert!(s.get("session_id").is_none());
        assert!(s.get("client_addr").is_none());
    }

    #[test]
    fn destructive_ops_require_confirmation() {
        assert!(require_yes(false, "x").is_err());
        assert!(require_yes(true, "x").is_ok());
    }
}
