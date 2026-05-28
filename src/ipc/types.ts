// Mirrors crates/mycharles-ipc/src/lib.rs. Hand-maintained for MVP; specta
// codegen is a follow-up task.

export interface ApiError {
  kind: string;
  message: string;
  details?: unknown;
}

export interface SessionDto {
  id: string;
  started_at: string;
  listen: string;
  status: string;
  ca_id: string;
}

export interface ProxyStatusDto {
  running: boolean;
  listen: string | null;
  captures_count: number;
}

export interface CaCertificateDto {
  id: string;
  serial: string;
  sha256_fp: string;
  subject: string;
  valid_from: string;
  valid_to: string;
  revoked_at: string | null;
}

export interface CaExportResult {
  format: string;
  data_base64: string | null;
  path: string | null;
  mime: string;
}

export interface DiscoveredDeviceDto {
  platform: "ios" | "android";
  serial: string;
  name: string;
}

export interface DeviceDto {
  id: string;
  platform: "ios" | "android";
  connection: "usb" | "wifi";
  serial: string;
  display_name: string;
  state: string;
  ca_installed_at: string | null;
  capabilities: Record<string, unknown>;
  last_error: string | null;
}

export interface HeaderDto {
  name: string;
  value: string;
}

export interface CaptureDto {
  id: string;
  session_id: string;
  started_at: string;
  ended_at: string | null;
  client_addr: string;
  server_host: string;
  server_port: number;
  scheme: "http" | "https";
  http_version: string;
  method: string;
  url_path: string;
  status: number | null;
  req_body_id: string | null;
  res_body_id: string | null;
  total_bytes: number;
  duration_ms: number | null;
  state: string;
  error_kind: string | null;
  req_headers?: HeaderDto[];
  res_headers?: HeaderDto[];
}

export interface CaptureBodyDto {
  mime: string | null;
  encoding: string;
  bytes_base64: string;
  truncated: boolean;
  total_size: number;
}

export interface FilterDto {
  id: string;
  name: string;
  query: string;
  color: string;
  pinned: boolean;
}

export interface RequestSpec {
  method: string;
  url: string;
  headers: HeaderDto[];
  body_base64?: string;
  body_text?: string;
  http_version?: string;
}

export interface ReplayRecordDto {
  id: string;
  source_capture_id: string | null;
  result_capture_id: string | null;
  created_at: string;
}

export interface PinningEventDto {
  capture_id: string;
  host: string;
  hint_kind: string;
}
