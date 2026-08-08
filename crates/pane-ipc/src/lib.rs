//! Wire-level DTOs and error type shared between Tauri commands and the SolidJS
//! frontend. Treated as a stable contract: changes here must be reflected in
//! `src/ipc/types.ts` (currently maintained by hand; specta codegen planned).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ------------------- Errors -------------------

/// `Deserialize` is not for the Tauri path — the frontend only ever receives
/// this. It exists for out-of-process clients (the CLI) that read `ApiError`
/// back off the control socket and map `kind` to an exit code.
#[derive(Debug, Serialize, Deserialize, Clone, thiserror::Error)]
#[error("{kind}: {message}")]
pub struct ApiError {
    pub kind: String,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

/// Every `ApiError::kind` value the backend produces.
///
/// These started life as bare string literals scattered across the command
/// modules. They are now public contract: the CLI switches on them to pick an
/// exit code, so a typo or a silent rename is a breaking change for callers
/// rather than a cosmetic one. Use the consts at every `to_api()` call site.
pub mod kinds {
    /// SQLite failure — query, migration, or lock.
    pub const DB: &str = "db";
    /// Filesystem read/write failure.
    pub const IO: &str = "io";
    /// The requested entity does not exist.
    pub const NOT_FOUND: &str = "not_found";
    /// A filter DSL string failed to parse. Distinct from [`DB`] so callers can
    /// tell "your query is malformed" from "the database is unhappy".
    pub const FILTER_PARSE: &str = "filter_parse";
    /// Host/port string could not be parsed into a `SocketAddr`.
    pub const INVALID_ADDR: &str = "invalid_addr";
    /// Proxy engine failed to bind or start.
    pub const ENGINE_START: &str = "engine_start";
    /// Proxy engine failed to shut down cleanly.
    pub const ENGINE_STOP: &str = "engine_stop";
    /// An operation needing a live proxy was called while it was stopped.
    pub const PROXY_NOT_RUNNING: &str = "proxy_not_running";
    /// No CA certificate is present in storage.
    pub const NO_CA: &str = "no_ca";
    /// CA rotation failed.
    pub const ROTATE_FAILED: &str = "rotate_failed";
    /// Export to curl/HAR/PEM/mobileconfig failed.
    pub const EXPORT_FAILED: &str = "export_failed";
    /// Writing an export to disk failed.
    pub const WRITE: &str = "write";
    /// base64 or text decode failed.
    pub const DECODE: &str = "decode";
    /// Replay request could not be sent.
    pub const REPLAY_FAILED: &str = "replay_failed";
    /// Required external tooling (adb, libimobiledevice) is missing.
    pub const TOOLING_MISSING: &str = "tooling_missing";
    /// An `adb` invocation failed.
    pub const ADB: &str = "adb";
    /// Pairing an iOS device failed.
    pub const IOS_ADD_FAILED: &str = "ios_add_failed";
    /// Pairing an Android device failed.
    pub const ANDROID_ADD_FAILED: &str = "android_add_failed";
    /// Unpairing a device failed.
    pub const REMOVE_FAILED: &str = "remove_failed";
    /// Spawning the `adb logcat` child failed.
    pub const LOGCAT_SPAWN: &str = "logcat_spawn";
    /// Creating a webview window failed. GUI-only.
    pub const WINDOW_BUILD: &str = "window_build";
    /// Enabling "Capture this Mac" failed.
    pub const HOST_CAPTURE_ENABLE: &str = "host_capture_enable";
    /// Disabling "Capture this Mac" failed.
    pub const HOST_CAPTURE_DISABLE: &str = "host_capture_disable";
}

// ------------------- Sessions / Proxy -------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyStartArgs {
    pub host: Option<String>,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDto {
    pub id: Uuid,
    pub started_at: String,
    pub listen: String,
    pub status: String,
    pub ca_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyStatusDto {
    pub running: bool,
    pub listen: Option<String>,
    pub captures_count: u64,
}

// ------------------- CA -------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaCertificateDto {
    pub id: Uuid,
    pub serial: String,
    pub sha256_fp: String,
    pub subject: String,
    pub valid_from: String,
    pub valid_to: String,
    pub revoked_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaExportArgs {
    pub format: String, // "pem" | "der" | "qr" | "mobileconfig"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaExportResult {
    pub format: String,
    pub data_base64: Option<String>,
    pub path: Option<String>,
    pub mime: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaSaveArgs {
    pub format: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaSaveResult {
    pub path: String,
    pub bytes_written: u64,
}

// ------------------- Devices -------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddDeviceArgs {
    pub serial: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveDeviceArgs {
    pub id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveDeviceResult {
    pub cleaned: bool,
    pub pending_cleanup: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredDeviceDto {
    pub platform: String, // "ios" | "android"
    pub serial: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AndroidToolingStatusDto {
    pub ok: bool,
    pub adb_path: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceDto {
    pub id: Uuid,
    pub platform: String,
    pub connection: String, // "usb" | "wifi"
    pub serial: String,
    pub display_name: String,
    pub state: String,
    pub ca_installed_at: Option<String>,
    pub capabilities: serde_json::Value,
    pub last_error: Option<String>,
}

// ------------------- Host capture ("Capture this Mac") -------------------

/// Was an ad-hoc `#[derive(Serialize)]` local to `commands/host.rs`. Promoted
/// here so out-of-process clients can deserialize it like every other DTO.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostCaptureStatusDto {
    pub enabled: bool,
    pub service: Option<String>,
}

// ------------------- Captures -------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListCapturesArgs {
    pub filter: Option<String>,
    pub limit: u32,
    pub before: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderDto {
    pub name: String,
    pub value: String,
}

/// One persisted logcat line, as returned to the Logcat window. Field names
/// mirror the frontend `LogEntry` (plus `id`/`created_at`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogcatRowDto {
    pub id: i64,
    pub created_at: i64,
    /// Device timestamp "MM-DD HH:MM:SS.mmm" (display only).
    pub timestamp: String,
    pub pid: u32,
    pub tid: u32,
    /// Lowercase level union ("verbose".."silent"), matching `LogEntry.level`.
    pub level: String,
    pub tag: String,
    pub message: String,
}

/// `filter` is the raw DSL string (tag/msg/level/pid/regex/bareword);
/// `include_pids`/`exclude_pids` carry the frontend-resolved `app:` PIDs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogcatQueryArgs {
    pub serial: String,
    pub filter: Option<String>,
    pub include_pids: Vec<u32>,
    pub exclude_pids: Vec<u32>,
    pub limit: u32,
}

/// "Load older on scroll-up": fetch the newest `limit` rows *older* than
/// `before_id`, matching the same filter + resolved `app:` PIDs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogcatQueryOlderArgs {
    pub serial: String,
    pub filter: Option<String>,
    pub include_pids: Vec<u32>,
    pub exclude_pids: Vec<u32>,
    pub before_id: i64,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogcatNewCountArgs {
    pub serial: String,
    pub filter: Option<String>,
    pub include_pids: Vec<u32>,
    pub exclude_pids: Vec<u32>,
    pub after_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogcatClearArgs {
    pub serial: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogcatExportArgs {
    pub serial: String,
    pub filter: Option<String>,
    pub include_pids: Vec<u32>,
    pub exclude_pids: Vec<u32>,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureDto {
    pub id: Uuid,
    pub session_id: Uuid,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub client_addr: String,
    pub server_host: String,
    pub server_port: u16,
    pub scheme: String,
    pub http_version: String,
    pub method: String,
    pub url_path: String,
    pub status: Option<u16>,
    pub req_body_id: Option<Uuid>,
    pub res_body_id: Option<Uuid>,
    pub total_bytes: u64,
    pub duration_ms: Option<u64>,
    pub state: String,
    pub error_kind: Option<String>,
    /// Persisted device-row id of the device this capture came from, or `None`
    /// for old captures, iOS, and connections on unattributed proxy ports.
    pub device_id: Option<String>,
    /// The rule that served this response, when `state` is `stubbed` or
    /// `patched`. `None` for live responses and for captures recorded before
    /// this was tracked.
    ///
    /// `state` alone only says a mock answered; with a large rule library
    /// that is much weaker than knowing *which* one, because a run that
    /// matched the wrong rule still looks successful.
    #[serde(default)]
    pub matched_rule_id: Option<String>,
    /// Name of `matched_rule_id`, denormalized so a capture still identifies
    /// the rule that served it after that rule is renamed or deleted.
    #[serde(default)]
    pub matched_rule_name: Option<String>,
    pub req_headers: Option<Vec<HeaderDto>>,
    pub res_headers: Option<Vec<HeaderDto>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBodyArgs {
    pub body_id: Uuid,
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureBodyDto {
    pub mime: Option<String>,
    pub encoding: String,
    pub bytes_base64: String,
    pub truncated: bool,
    pub total_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClearArgs {
    pub older_than: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClearResult {
    pub deleted: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportOneArgs {
    pub id: Uuid,
    pub format: String, // "curl" | "har_single"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportOneResult {
    pub text: String,
    pub mime: String,
}

// ------------------- Replay -------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestSpec {
    pub method: String,
    pub url: String,
    pub headers: Vec<HeaderDto>,
    pub body_base64: Option<String>,
    pub body_text: Option<String>,
    pub http_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplaySendArgs {
    pub source_id: Option<Uuid>,
    pub request: RequestSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayRecordDto {
    pub id: Uuid,
    pub source_capture_id: Option<Uuid>,
    pub result_capture_id: Option<Uuid>,
    pub created_at: String,
}

// ------------------- Filters -------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveFilterArgs {
    pub id: Option<Uuid>,
    pub name: String,
    pub query: String,
    pub color: String,
    pub pinned: bool,
    /// Scope of the filter. "captures" (default) or "logcat".
    /// Migration V005 added this column; older clients omitting the
    /// field land in the "captures" bucket via serde default.
    #[serde(default = "default_filter_kind")]
    pub kind: String,
}

fn default_filter_kind() -> String {
    "captures".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterDto {
    pub id: Uuid,
    pub name: String,
    pub query: String,
    pub color: String,
    pub pinned: bool,
    pub kind: String,
}

// ------------------- Rules (response stubbing) -------------------

/// Name/value param matched against either the query string of a GET-style
/// request, or the top-level fields of a JSON request body. The matcher
/// stringifies JSON numbers/booleans before comparing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleParamDto {
    pub name: String,
    pub value: String,
}

/// One predicate condition on a request-body field. `path` is a dot/bracket
/// path into the JSON body (e.g. `amount`, `payment.details.sum`,
/// `items[0].price`); `op` is a comparison operator (`eq`, `ne`, `gt`, `gte`,
/// `lt`, `lte`, `contains`); `value` is the right-hand side as a string
/// (numeric ops coerce both sides to f64). Multiple conditions are AND-ed, so
/// a range is two rows (`gte 1000` + `lte 1500`). Nested-capable and typed,
/// unlike `match_params` (top-level, string-equality only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleConditionDto {
    pub path: String,
    pub op: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleHeaderDto {
    pub name: String,
    pub value: String,
}

/// One mutation applied by a `mode = "patch"` rule. `path` walks a virtual
/// response tree (`status`, `headers.<Name>`, `body.<dot.path>`).
/// `value` is parsed as JSON; if that fails it's treated as a plain string.
/// `delete` ignores `value`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RulePatchOpDto {
    pub op: String, // "set" | "delete" | "append"
    pub path: String,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleDto {
    pub id: Uuid,
    pub name: String,
    pub enabled: bool,
    pub priority: i64,
    pub collection_id: Option<Uuid>,

    /// "stub" (default — replace whole response) or "patch" (forward upstream,
    /// then apply `patches`).
    pub mode: String,
    pub patches: Vec<RulePatchOpDto>,

    pub match_host_glob: Option<String>,
    pub match_method: Option<String>,
    pub match_path_glob: Option<String>,
    pub match_params: Vec<RuleParamDto>,
    /// Optional JSON the engine deep-subset-matches against the request
    /// body (nested-capable, unlike `match_params` which only sees
    /// top-level scalars). `None`/blank = don't match on the body.
    pub match_req_body: Option<String>,
    /// Predicate conditions on request-body fields (range / comparison /
    /// substring). AND-ed together and with the other matchers.
    #[serde(default)]
    pub match_conditions: Vec<RuleConditionDto>,

    pub res_status: u16,
    pub res_headers: Vec<RuleHeaderDto>,
    pub res_body_id: Option<Uuid>,
    /// Hint for the UI: mime + size of the referenced body if present.
    pub res_body_mime: Option<String>,
    pub res_body_size: u64,
    pub res_delay_ms: u64,

    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleUpsertArgs {
    pub id: Option<Uuid>,
    pub name: String,
    pub enabled: bool,
    pub priority: i64,
    pub collection_id: Option<Uuid>,

    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub patches: Vec<RulePatchOpDto>,

    pub match_host_glob: Option<String>,
    pub match_method: Option<String>,
    pub match_path_glob: Option<String>,
    pub match_params: Vec<RuleParamDto>,
    /// Optional JSON deep-subset-matched against the request body. A
    /// blank/whitespace string is treated as `None` (no body matching).
    #[serde(default)]
    pub match_req_body: Option<String>,
    #[serde(default)]
    pub match_conditions: Vec<RuleConditionDto>,

    pub res_status: u16,
    pub res_headers: Vec<RuleHeaderDto>,
    /// Either reference an existing capture body…
    pub res_body_id: Option<Uuid>,
    /// …or inline new bytes (base64). If both are set, body_id wins.
    pub res_body_base64: Option<String>,
    pub res_body_mime: Option<String>,
    pub res_delay_ms: u64,
}

fn default_mode() -> String {
    "stub".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleSetEnabledArgs {
    pub id: Uuid,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleSetPriorityArgs {
    pub id: Uuid,
    pub priority: i64,
}

/// Which rules a bulk enable/disable covers.
///
/// Externally tagged so the wire form reads `{"kind":"collection","id":"…"}` —
/// a bare optional id could not tell "every rule" from "the ungrouped ones",
/// and getting that wrong silently flips the whole library.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuleBulkScope {
    /// Every rule in the library.
    All,
    /// Every rule in one collection.
    Collection { id: Uuid },
    /// Every rule that belongs to no collection.
    Ungrouped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RulesSetEnabledBulkArgs {
    pub enabled: bool,
    pub scope: RuleBulkScope,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RulesSetEnabledBulkResult {
    /// Rules the scope covered.
    pub matched: u64,
    /// Of those, how many actually changed state. Reported separately so a
    /// caller can tell "nothing to do" from "scope matched nothing" — the
    /// second is usually a typo'd selector, the first is success.
    pub changed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleCollectionDto {
    pub id: Uuid,
    pub name: String,
    pub enabled: bool,
    pub priority: i64,
    pub rule_count: u64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionUpsertArgs {
    pub id: Option<Uuid>,
    pub name: String,
    pub enabled: bool,
    pub priority: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionSetEnabledArgs {
    pub id: Uuid,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionSetPriorityArgs {
    pub id: Uuid,
    pub priority: i64,
}

// ------------------- Pinning event -------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinningEventDto {
    pub capture_id: Uuid,
    pub host: String,
    pub hint_kind: String,
}
