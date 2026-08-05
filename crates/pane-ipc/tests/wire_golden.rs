//! Golden snapshot of the wire contract.
//!
//! `crates/pane-ipc/src/lib.rs` and `src/ipc/types.ts` are kept in sync **by
//! hand** — there is no specta codegen. Until there is, a renamed or dropped
//! field on the Rust side fails silently: the frontend keeps compiling and
//! only breaks at runtime, and out-of-process clients (the CLI, the MCP
//! server) break without anyone noticing at all.
//!
//! This test pins the serialized shape of every DTO. It does not check that
//! the TypeScript mirror agrees — it checks that **you knew you were changing
//! the contract**. When it fails, the fix is:
//!
//!   1. confirm the change is intentional,
//!   2. update `src/ipc/types.ts` to match,
//!   3. update the expectation here.
//!
//! Only key names and JSON value *shapes* are asserted, not the sample values,
//! so this stays readable and doesn't churn on unrelated edits.

use pane_ipc::*;
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

/// Serialize `v` and reduce it to a `{field: type-tag}` map so the assertion
/// reads as a schema rather than a fixture. Nested objects recurse; arrays
/// collapse to the shape of their first element (or `"array<empty>"`).
fn shape<T: Serialize>(v: &T) -> Value {
    fn walk(v: &Value) -> Value {
        match v {
            Value::Object(map) => {
                Value::Object(map.iter().map(|(k, v)| (k.clone(), walk(v))).collect())
            }
            Value::Array(items) => match items.first() {
                Some(first) => json!([walk(first)]),
                None => Value::String("array<empty>".into()),
            },
            Value::String(_) => Value::String("string".into()),
            Value::Number(n) => Value::String(if n.is_f64() { "f64" } else { "number" }.into()),
            Value::Bool(_) => Value::String("bool".into()),
            // `null` is indistinguishable from "optional field, absent value"
            // at this level. Every DTO below sets its Option fields to Some so
            // the real type is what gets pinned; a bare `null` here means the
            // field is *always* null, which is itself worth noticing.
            Value::Null => Value::String("null".into()),
        }
    }
    walk(&serde_json::to_value(v).expect("DTO must serialize"))
}

fn uuid() -> Uuid {
    Uuid::nil()
}

fn headers() -> Vec<HeaderDto> {
    vec![HeaderDto {
        name: "content-type".into(),
        value: "application/json".into(),
    }]
}

#[test]
fn api_error_shape() {
    let e = ApiError {
        kind: kinds::DB.into(),
        message: "boom".into(),
        details: Some(json!({"any": "json"})),
    };
    assert_eq!(
        shape(&e),
        json!({"kind": "string", "message": "string", "details": {"any": "string"}})
    );
}

/// `ApiError` must round-trip: the CLI reads it back off the control socket
/// and maps `kind` to an exit code. A `Serialize`-only DTO would compile fine
/// here and fail at the client.
#[test]
fn api_error_round_trips() {
    let e = ApiError {
        kind: kinds::NOT_FOUND.into(),
        message: "no such capture".into(),
        details: None,
    };
    let back: ApiError = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
    assert_eq!(back.kind, kinds::NOT_FOUND);
    assert_eq!(back.message, "no such capture");
    assert!(back.details.is_none());
}

/// Every `kinds` const is distinct and snake_case. These strings are public
/// contract — the CLI switches on them — so a copy-paste duplicate would
/// silently collapse two error classes into one exit code.
#[test]
fn error_kinds_are_unique_and_snake_case() {
    let all = [
        kinds::DB,
        kinds::IO,
        kinds::NOT_FOUND,
        kinds::FILTER_PARSE,
        kinds::INVALID_ADDR,
        kinds::ENGINE_START,
        kinds::ENGINE_STOP,
        kinds::PROXY_NOT_RUNNING,
        kinds::NO_CA,
        kinds::ROTATE_FAILED,
        kinds::EXPORT_FAILED,
        kinds::WRITE,
        kinds::DECODE,
        kinds::REPLAY_FAILED,
        kinds::TOOLING_MISSING,
        kinds::ADB,
        kinds::IOS_ADD_FAILED,
        kinds::ANDROID_ADD_FAILED,
        kinds::REMOVE_FAILED,
        kinds::LOGCAT_SPAWN,
        kinds::WINDOW_BUILD,
        kinds::HOST_CAPTURE_ENABLE,
        kinds::HOST_CAPTURE_DISABLE,
    ];
    let mut sorted = all.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), all.len(), "duplicate ApiError kind");
    for k in all {
        assert!(
            !k.is_empty() && k.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "kind {k:?} is not snake_case"
        );
    }
}

#[test]
fn capture_dto_shape() {
    let c = CaptureDto {
        id: uuid(),
        session_id: uuid(),
        started_at: "2026-08-05T12:31:04.812Z".into(),
        ended_at: Some("2026-08-05T12:31:06.016Z".into()),
        client_addr: "127.0.0.1:51234".into(),
        server_host: "api.example.com".into(),
        server_port: 443,
        scheme: "https".into(),
        http_version: "HTTP/1.1".into(),
        method: "POST".into(),
        url_path: "/v1/pay".into(),
        status: Some(500),
        req_body_id: Some(uuid()),
        res_body_id: Some(uuid()),
        total_bytes: 1132,
        duration_ms: Some(1204),
        state: "completed".into(),
        error_kind: Some("upstream".into()),
        device_id: Some(uuid().to_string()),
        req_headers: Some(headers()),
        res_headers: Some(headers()),
    };
    let header_shape = json!([{"name": "string", "value": "string"}]);
    assert_eq!(
        shape(&c),
        json!({
            "id": "string", "session_id": "string",
            "started_at": "string", "ended_at": "string",
            "client_addr": "string", "server_host": "string", "server_port": "number",
            "scheme": "string", "http_version": "string",
            "method": "string", "url_path": "string", "status": "number",
            "req_body_id": "string", "res_body_id": "string",
            "total_bytes": "number", "duration_ms": "number",
            "state": "string", "error_kind": "string", "device_id": "string",
            "req_headers": header_shape, "res_headers": header_shape,
        })
    );
}

#[test]
fn capture_body_dto_shape() {
    let b = CaptureBodyDto {
        mime: Some("application/json".into()),
        encoding: "identity".into(),
        bytes_base64: "e30=".into(),
        truncated: true,
        total_size: 1132,
    };
    assert_eq!(
        shape(&b),
        json!({
            "mime": "string", "encoding": "string", "bytes_base64": "string",
            "truncated": "bool", "total_size": "number",
        })
    );
}

#[test]
fn proxy_dtos_shape() {
    let s = SessionDto {
        id: uuid(),
        started_at: "2026-08-05T12:28:41Z".into(),
        listen: "127.0.0.1:8888".into(),
        status: "running".into(),
        ca_id: uuid(),
    };
    assert_eq!(
        shape(&s),
        json!({
            "id": "string", "started_at": "string", "listen": "string",
            "status": "string", "ca_id": "string",
        })
    );

    let st = ProxyStatusDto {
        running: true,
        listen: Some("127.0.0.1:8888".into()),
        captures_count: 1284,
    };
    assert_eq!(
        shape(&st),
        json!({"running": "bool", "listen": "string", "captures_count": "number"})
    );
}

#[test]
fn device_dtos_shape() {
    let d = DeviceDto {
        id: uuid(),
        platform: "android".into(),
        connection: "usb".into(),
        serial: "R5CT30ABCDE".into(),
        display_name: "Galaxy S23".into(),
        state: "ready".into(),
        ca_installed_at: Some("2026-08-05T12:29:02Z".into()),
        capabilities: json!({"root": false}),
        last_error: Some("adb timeout".into()),
    };
    assert_eq!(
        shape(&d),
        json!({
            "id": "string", "platform": "string", "connection": "string",
            "serial": "string", "display_name": "string", "state": "string",
            "ca_installed_at": "string", "capabilities": {"root": "bool"},
            "last_error": "string",
        })
    );

    let disc = DiscoveredDeviceDto {
        platform: "android".into(),
        serial: "R5CT30ABCDE".into(),
        name: "Galaxy S23".into(),
    };
    assert_eq!(
        shape(&disc),
        json!({"platform": "string", "serial": "string", "name": "string"})
    );
}

#[test]
fn ca_dto_shape() {
    let c = CaCertificateDto {
        id: uuid(),
        serial: "01".into(),
        sha256_fp: "d4:1a".into(),
        subject: "Pane Root CA".into(),
        valid_from: "2026-08-05T00:00:00Z".into(),
        valid_to: "2035-08-05T00:00:00Z".into(),
        revoked_at: Some("2026-09-01T00:00:00Z".into()),
    };
    assert_eq!(
        shape(&c),
        json!({
            "id": "string", "serial": "string", "sha256_fp": "string",
            "subject": "string", "valid_from": "string", "valid_to": "string",
            "revoked_at": "string",
        })
    );
}

#[test]
fn rule_dto_shape() {
    let r = RuleDto {
        id: uuid(),
        name: "orders-500".into(),
        enabled: true,
        priority: 0,
        collection_id: Some(uuid()),
        mode: "stub".into(),
        patches: vec![RulePatchOpDto {
            op: "set".into(),
            path: "body.a".into(),
            value: Some(json!(1)),
        }],
        match_host_glob: Some("api.example.com".into()),
        match_method: Some("GET".into()),
        match_path_glob: Some("/v1/orders*".into()),
        match_params: vec![RuleParamDto {
            name: "page".into(),
            value: "1".into(),
        }],
        match_req_body: Some("{}".into()),
        match_conditions: vec![RuleConditionDto {
            path: "amount".into(),
            op: "gte".into(),
            value: "1000".into(),
        }],
        res_status: 500,
        res_headers: vec![RuleHeaderDto {
            name: "content-type".into(),
            value: "application/json".into(),
        }],
        res_body_id: Some(uuid()),
        res_body_mime: Some("application/json".into()),
        res_body_size: 58,
        res_delay_ms: 250,
        created_at: "2026-08-05T12:00:00Z".into(),
        updated_at: "2026-08-05T12:00:00Z".into(),
    };
    assert_eq!(
        shape(&r),
        json!({
            "id": "string", "name": "string", "enabled": "bool", "priority": "number",
            "collection_id": "string",
            "mode": "string",
            "patches": [{"op": "string", "path": "string", "value": "number"}],
            "match_host_glob": "string", "match_method": "string", "match_path_glob": "string",
            "match_params": [{"name": "string", "value": "string"}],
            "match_req_body": "string",
            "match_conditions": [{"path": "string", "op": "string", "value": "string"}],
            "res_status": "number",
            "res_headers": [{"name": "string", "value": "string"}],
            "res_body_id": "string", "res_body_mime": "string", "res_body_size": "number",
            "res_delay_ms": "number",
            "created_at": "string", "updated_at": "string",
        })
    );
}

#[test]
fn rule_collection_dto_shape() {
    let c = RuleCollectionDto {
        id: uuid(),
        name: "payments".into(),
        enabled: true,
        priority: 0,
        rule_count: 3,
        created_at: "2026-08-05T12:00:00Z".into(),
        updated_at: "2026-08-05T12:00:00Z".into(),
    };
    assert_eq!(
        shape(&c),
        json!({
            "id": "string", "name": "string", "enabled": "bool", "priority": "number",
            "rule_count": "number", "created_at": "string", "updated_at": "string",
        })
    );
}

#[test]
fn filter_dto_shape() {
    let f = FilterDto {
        id: uuid(),
        name: "errors".into(),
        query: "status:500..599".into(),
        color: "#ff0000".into(),
        pinned: true,
        kind: "captures".into(),
    };
    assert_eq!(
        shape(&f),
        json!({
            "id": "string", "name": "string", "query": "string",
            "color": "string", "pinned": "bool", "kind": "string",
        })
    );
}

#[test]
fn logcat_row_dto_shape() {
    let r = LogcatRowDto {
        id: 42,
        created_at: 1_754_395_864_812,
        timestamp: "08-05 12:31:04.812".into(),
        pid: 12844,
        tid: 12844,
        level: "error".into(),
        tag: "OkHttp".into(),
        message: "HTTP FAILED".into(),
    };
    assert_eq!(
        shape(&r),
        json!({
            "id": "number", "created_at": "number", "timestamp": "string",
            "pid": "number", "tid": "number", "level": "string",
            "tag": "string", "message": "string",
        })
    );
}

#[test]
fn replay_dtos_shape() {
    let rec = ReplayRecordDto {
        id: uuid(),
        source_capture_id: Some(uuid()),
        result_capture_id: Some(uuid()),
        created_at: "2026-08-05T12:00:00Z".into(),
    };
    assert_eq!(
        shape(&rec),
        json!({
            "id": "string", "source_capture_id": "string",
            "result_capture_id": "string", "created_at": "string",
        })
    );

    let spec = RequestSpec {
        method: "POST".into(),
        url: "https://api.example.com/v1/pay".into(),
        headers: headers(),
        body_base64: Some("e30=".into()),
        body_text: Some("{}".into()),
        http_version: Some("HTTP/1.1".into()),
    };
    assert_eq!(
        shape(&spec),
        json!({
            "method": "string", "url": "string",
            "headers": [{"name": "string", "value": "string"}],
            "body_base64": "string", "body_text": "string", "http_version": "string",
        })
    );
}

#[test]
fn pinning_event_dto_shape() {
    let p = PinningEventDto {
        capture_id: uuid(),
        host: "api.example.com".into(),
        hint_kind: "alpn_h2_reset".into(),
    };
    assert_eq!(
        shape(&p),
        json!({"capture_id": "string", "host": "string", "hint_kind": "string"})
    );
}

/// Arg structs travel the other direction (client → backend) and are the
/// `params` payload of every control-socket request, so they are contract too.
#[test]
fn arg_structs_deserialize_from_wire_json() {
    let a: ListCapturesArgs =
        serde_json::from_value(json!({"filter": "status:500", "limit": 50, "before": null}))
            .unwrap();
    assert_eq!(a.limit, 50);
    assert_eq!(a.filter.as_deref(), Some("status:500"));

    let b: GetBodyArgs =
        serde_json::from_value(json!({"body_id": Uuid::nil(), "max_bytes": 8192})).unwrap();
    assert_eq!(b.max_bytes, Some(8192));

    let p: ProxyStartArgs = serde_json::from_value(json!({"host": null, "port": 8888})).unwrap();
    assert_eq!(p.port, Some(8888));

    // `kind` defaults to "captures" for clients predating the logcat split.
    let f: SaveFilterArgs = serde_json::from_value(json!({
        "id": null, "name": "errors", "query": "status:500", "color": "#f00", "pinned": false
    }))
    .unwrap();
    assert_eq!(f.kind, "captures");

    // `mode` defaults to "stub", and the three body-matching fields are all
    // optional — the CLI's `rules mock` sugar relies on omitting them.
    let r: RuleUpsertArgs = serde_json::from_value(json!({
        "id": null, "name": "m", "enabled": true, "priority": 0, "collection_id": null,
        "match_host_glob": "api.example.com", "match_method": null, "match_path_glob": null,
        "match_params": [], "res_status": 500, "res_headers": [],
        "res_body_id": null, "res_body_base64": null, "res_body_mime": null, "res_delay_ms": 0
    }))
    .unwrap();
    assert_eq!(r.mode, "stub");
    assert!(r.patches.is_empty());
    assert!(r.match_conditions.is_empty());
    assert!(r.match_req_body.is_none());

    let l: LogcatQueryArgs = serde_json::from_value(json!({
        "serial": "R5CT30ABCDE", "filter": "level:E",
        "include_pids": [12844], "exclude_pids": [], "limit": 200
    }))
    .unwrap();
    assert_eq!(l.include_pids, vec![12844]);
}
