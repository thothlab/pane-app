//! Wire-level DTOs and error type shared between Tauri commands and the SolidJS
//! frontend. Treated as a stable contract: changes here must be reflected in
//! `src/ipc/types.ts` (currently maintained by hand; specta codegen planned).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ------------------- Errors -------------------

#[derive(Debug, Serialize, Clone, thiserror::Error)]
#[error("{kind}: {message}")]
pub struct ApiError {
    pub kind: String,
    pub message: String,
    pub details: Option<serde_json::Value>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterDto {
    pub id: Uuid,
    pub name: String,
    pub query: String,
    pub color: String,
    pub pinned: bool,
}

// ------------------- Rules (response stubbing) -------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleQueryParamDto {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleHeaderDto {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleDto {
    pub id: Uuid,
    pub name: String,
    pub enabled: bool,
    pub priority: i64,

    pub match_host_glob: Option<String>,
    pub match_method: Option<String>,
    pub match_path_glob: Option<String>,
    pub match_query: Vec<RuleQueryParamDto>,

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

    pub match_host_glob: Option<String>,
    pub match_method: Option<String>,
    pub match_path_glob: Option<String>,
    pub match_query: Vec<RuleQueryParamDto>,

    pub res_status: u16,
    pub res_headers: Vec<RuleHeaderDto>,
    /// Either reference an existing capture body…
    pub res_body_id: Option<Uuid>,
    /// …or inline new bytes (base64). If both are set, body_id wins.
    pub res_body_base64: Option<String>,
    pub res_body_mime: Option<String>,
    pub res_delay_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleSetEnabledArgs {
    pub id: Uuid,
    pub enabled: bool,
}

// ------------------- Pinning event -------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinningEventDto {
    pub capture_id: Uuid,
    pub host: String,
    pub hint_kind: String,
}
