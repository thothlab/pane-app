//! Wire op → `Core` method.
//!
//! One place that names every remotely-callable operation. Adding an op here
//! makes it reachable from the CLI and the MCP server at once.

use std::sync::Arc;

use pane_core::Core;
use pane_ipc::{kinds, ApiError};
use serde_json::Value;
use uuid::Uuid;

use crate::protocol::Pong;

/// Every op this server understands. Ordered by namespace; used by `pane
/// schema` and by the "unknown op" error so a typo lists the alternatives.
pub const OPS: &[&str] = &[
    "ping",
    "proxy.start",
    "proxy.stop",
    "proxy.status",
    "captures.list",
    "captures.count",
    "captures.get",
    "captures.body",
    "captures.clear",
    "captures.export",
    "rules.list",
    "rules.get",
    "rules.upsert",
    "rules.delete",
    "rules.set_enabled",
    "rules.set_enabled_bulk",
    "rules.set_priority",
    "collections.list",
    "collections.upsert",
    "collections.delete",
    "collections.set_enabled",
    "collections.set_priority",
    "devices.list",
    "devices.attached",
    "devices.add_android",
    "devices.add_ios",
    "devices.get",
    "devices.remove",
    "devices.tooling_status",
    "ca.current",
    "ca.export",
    "ca.rotate",
    "filters.list",
    "filters.save",
    "filters.delete",
    "replay.send",
    "logcat.start",
    "logcat.stop",
    "logcat.active",
    "logcat.query",
    "logcat.query_older",
    "logcat.new_count",
    "logcat.clear",
    "logcat.export",
    "logcat.pid_names",
    "host.enable",
    "host.disable",
    "host.status",
    "events.subscribe",
    "events.unsubscribe",
];

fn bad_params(e: impl std::fmt::Display) -> ApiError {
    pane_core::api_err("bad_params", e.to_string())
}

/// Deserialize `params` into the arg struct an op expects.
fn parse<T: serde::de::DeserializeOwned>(params: Value) -> Result<T, ApiError> {
    serde_json::from_value(params).map_err(bad_params)
}

/// Pull a required UUID field out of `params`, accepting either a bare string
/// (`"…"`) or `{"id": "…"}` so callers can use whichever is natural.
fn parse_id(params: &Value, field: &str) -> Result<Uuid, ApiError> {
    let raw = params
        .as_str()
        .or_else(|| params.get(field).and_then(|v| v.as_str()))
        .ok_or_else(|| bad_params(format!("missing `{field}`")))?;
    Uuid::parse_str(raw).map_err(bad_params)
}

fn ok<T: serde::Serialize>(v: T) -> Result<Value, ApiError> {
    serde_json::to_value(v).map_err(|e| pane_core::api_err(kinds::DB, e.to_string()))
}

/// Run one non-streaming op. `events.*` are handled by the connection loop,
/// which owns the subscription tasks, so they are rejected here.
pub async fn dispatch(core: &Arc<Core>, op: &str, params: Value) -> Result<Value, ApiError> {
    match op {
        "ping" => ok(Pong {
            protocol: crate::protocol::PROTOCOL_VERSION,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            pid: std::process::id(),
        }),

        // ---- proxy ----
        "proxy.start" => ok(core.proxy_start(parse(params)?).await?),
        "proxy.stop" => ok(core.proxy_stop().await?),
        "proxy.status" => ok(core.proxy_status().await?),

        // ---- captures ----
        "captures.list" => ok(core.captures_list(parse(params)?).await?),
        "captures.count" => ok(core.captures_count().await?),
        "captures.get" => ok(core.capture_get(parse_id(&params, "id")?).await?),
        "captures.body" => {
            let args: pane_ipc::GetBodyArgs = parse(params)?;
            ok(core.capture_body(args.body_id, args.max_bytes).await?)
        }
        "captures.clear" => {
            let args: pane_ipc::ClearArgs =
                parse(params).unwrap_or(pane_ipc::ClearArgs { older_than: None });
            ok(core.captures_clear(args.older_than).await?)
        }
        "captures.export" => {
            let args: pane_ipc::ExportOneArgs = parse(params)?;
            ok(core.capture_export(args.id, &args.format).await?)
        }

        // ---- rules ----
        "rules.list" => ok(core.rules_list().await?),
        "rules.get" => ok(core.rule_get(parse_id(&params, "id")?).await?),
        "rules.upsert" => ok(core.rule_upsert(parse(params)?).await?),
        "rules.delete" => ok(core.rule_delete(parse_id(&params, "id")?).await?),
        "rules.set_enabled" => ok(core.rule_set_enabled(parse(params)?).await?),
        "rules.set_enabled_bulk" => ok(core.rules_set_enabled_bulk(parse(params)?).await?),
        "rules.set_priority" => ok(core.rule_set_priority(parse(params)?).await?),

        // ---- collections ----
        "collections.list" => ok(core.collections_list().await?),
        "collections.upsert" => ok(core.collection_upsert(parse(params)?).await?),
        "collections.delete" => ok(core.collection_delete(parse_id(&params, "id")?).await?),
        "collections.set_enabled" => ok(core.collection_set_enabled(parse(params)?).await?),
        "collections.set_priority" => ok(core.collection_set_priority(parse(params)?).await?),

        // ---- devices ----
        "devices.list" => ok(core.devices_list().await?),
        "devices.attached" => ok(core.devices_attached().await?),
        "devices.add_android" => {
            let args: pane_ipc::AddDeviceArgs = parse(params)?;
            ok(core.device_add_android(&args.serial).await?)
        }
        "devices.add_ios" => {
            let args: pane_ipc::AddDeviceArgs = parse(params)?;
            ok(core.device_add_ios(&args.serial).await?)
        }
        "devices.get" => ok(core.device_get(parse_id(&params, "id")?).await?),
        "devices.remove" => ok(core.device_remove(parse_id(&params, "id")?).await?),
        "devices.tooling_status" => ok(core.android_tooling_status().await?),

        // ---- ca ----
        "ca.current" => ok(core.ca_current().await?),
        "ca.export" => {
            let args: pane_ipc::CaExportArgs = parse(params)?;
            ok(core.ca_export(&args.format).await?)
        }
        "ca.rotate" => ok(core.ca_rotate().await?),

        // ---- filters ----
        "filters.list" => {
            let kind = params
                .get("kind")
                .and_then(|v| v.as_str())
                .map(String::from);
            ok(core.filters_list(kind.as_deref()).await?)
        }
        "filters.save" => ok(core.filter_save(parse(params)?).await?),
        "filters.delete" => ok(core.filter_delete(parse_id(&params, "id")?).await?),

        // ---- replay ----
        "replay.send" => ok(core.replay_send(parse(params)?).await?),

        // ---- logcat ----
        "logcat.start" => {
            let serial = require_serial(&params)?;
            ok(serde_json::json!({ "reused": core.logcat_start(&serial)?.reused }))
        }
        "logcat.stop" => {
            core.logcat_stop(&require_serial(&params)?);
            ok(serde_json::json!({ "stopped": true }))
        }
        "logcat.active" => ok(core.logcat_active_serials()),
        "logcat.query" => ok(core.logcat_query(parse(params)?).await?),
        "logcat.query_older" => ok(core.logcat_query_older(parse(params)?).await?),
        "logcat.new_count" => ok(core.logcat_new_count(parse(params)?).await?),
        "logcat.clear" => ok(core.logcat_clear(&require_serial(&params)?).await?),
        "logcat.export" => ok(core.logcat_export(parse(params)?).await?),
        "logcat.pid_names" => ok(core.android_pid_names(&require_serial(&params)?).await?),

        // ---- host capture ----
        "host.enable" => ok(core.host_capture_enable().await?),
        "host.disable" => ok(core.host_capture_disable().await?),
        "host.status" => ok(core.host_capture_status().await?),

        "events.subscribe" | "events.unsubscribe" => Err(pane_core::api_err(
            "bad_params",
            "events.* are handled by the connection, not the dispatcher",
        )),

        other => Err(pane_core::api_err(
            "unknown_op",
            format!("unknown op `{other}`; try one of: {}", OPS.join(", ")),
        )),
    }
}

fn require_serial(params: &Value) -> Result<String, ApiError> {
    params
        .as_str()
        .or_else(|| params.get("serial").and_then(|v| v.as_str()))
        .map(String::from)
        .ok_or_else(|| bad_params("missing `serial`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_list_has_no_duplicates() {
        let mut sorted = OPS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), OPS.len(), "duplicate op name");
    }

    #[test]
    fn every_op_is_namespaced_or_a_known_bare_verb() {
        for op in OPS {
            assert!(
                op.contains('.') || *op == "ping",
                "op `{op}` should be namespaced"
            );
        }
    }

    #[test]
    fn id_params_accept_both_shapes() {
        let bare = serde_json::json!("00000000-0000-0000-0000-000000000000");
        let wrapped = serde_json::json!({"id": "00000000-0000-0000-0000-000000000000"});
        assert_eq!(parse_id(&bare, "id").unwrap(), Uuid::nil());
        assert_eq!(parse_id(&wrapped, "id").unwrap(), Uuid::nil());
        assert!(parse_id(&serde_json::json!({}), "id").is_err());
    }

    #[test]
    fn serial_params_accept_both_shapes() {
        assert_eq!(require_serial(&serde_json::json!("R5CT")).unwrap(), "R5CT");
        assert_eq!(
            require_serial(&serde_json::json!({"serial": "R5CT"})).unwrap(),
            "R5CT"
        );
        assert!(require_serial(&serde_json::json!({})).is_err());
    }
}
