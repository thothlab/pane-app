// Mirrors crates/pane-ipc/src/lib.rs. Hand-maintained for MVP; specta
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

export interface CaSaveResult {
  path: string;
  bytes_written: number;
}

export interface DiscoveredDeviceDto {
  platform: "ios" | "android";
  serial: string;
  name: string;
}

export interface AndroidToolingStatusDto {
  ok: boolean;
  adb_path: string | null;
  error: string | null;
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
  /** "completed" | "stubbed" | "patched" | "error". */
  state: string;
  error_kind: string | null;
  /** Evidence behind `error_kind`: the TLS alert, the I/O error, or why the
   *  host is being tunnelled. Null on success and on pre-0.2.10 captures. */
  error_detail: string | null;
  device_id: string | null;
  /**
   * The rule that served this response, set when `state` is "stubbed" or
   * "patched". Null for live responses and for captures recorded before this
   * was tracked.
   */
  matched_rule_id?: string | null;
  /** Name of `matched_rule_id`, denormalized so it survives rule deletion. */
  matched_rule_name?: string | null;
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

export type FilterKind = "captures" | "logcat";

export interface FilterDto {
  id: string;
  name: string;
  query: string;
  color: string;
  pinned: boolean;
  kind: FilterKind;
}

/** One persisted logcat row from the DB. Mirrors the backend LogcatRowDto;
 *  the `timestamp`/`pid`/`tid`/`level`/`tag`/`message` fields line up with
 *  the in-memory `LogEntry` so the table renders either interchangeably. */
export interface LogcatRowDto {
  id: number;
  created_at: number;
  timestamp: string;
  pid: number;
  tid: number;
  level: "verbose" | "debug" | "info" | "warn" | "error" | "fatal" | "silent";
  tag: string;
  message: string;
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

export interface RuleParamDto {
  name: string;
  value: string;
}

export interface RuleHeaderDto {
  name: string;
  value: string;
}

export type RulePatchOpKind = "set" | "delete" | "append";

export interface RulePatchOpDto {
  op: RulePatchOpKind;
  path: string;
  value?: unknown;
}

export type RuleMode = "stub" | "patch";

export type RuleConditionOp = "eq" | "ne" | "gt" | "gte" | "lt" | "lte" | "contains";

/** A predicate on a request-body field: `path` (dot/bracket into the JSON
 * body), `op` (comparison), `value` (right-hand side as string). AND-ed with
 * everything else; a numeric range is two rows (gte + lte). */
export interface RuleConditionDto {
  path: string;
  op: RuleConditionOp;
  value: string;
}

export interface RuleDto {
  id: string;
  name: string;
  enabled: boolean;
  priority: number;
  collection_id: string | null;
  mode: RuleMode;
  patches: RulePatchOpDto[];
  match_host_glob: string | null;
  match_method: string | null;
  match_path_glob: string | null;
  match_params: RuleParamDto[];
  /** Optional JSON deep-subset-matched against the request body (nested-
   * capable). null/blank = don't match on the body. */
  match_req_body: string | null;
  match_conditions: RuleConditionDto[];
  /** Free-form labels. They carry no behaviour — the engine never reads
   * them, so a tag can never be why a mock did or didn't fire. Normalised
   * on write: trimmed, blanks dropped, case-insensitive duplicates removed. */
  tags: string[];
  res_status: number;
  res_headers: RuleHeaderDto[];
  res_body_id: string | null;
  res_body_mime: string | null;
  res_body_size: number;
  res_delay_ms: number;
  created_at: string;
  updated_at: string;
}

export interface RuleUpsertArgs {
  id?: string | null;
  name: string;
  enabled: boolean;
  priority: number;
  collection_id: string | null;
  mode: RuleMode;
  patches: RulePatchOpDto[];
  match_host_glob: string | null;
  match_method: string | null;
  match_path_glob: string | null;
  match_params: RuleParamDto[];
  match_req_body?: string | null;
  match_conditions?: RuleConditionDto[];
  /** Omitted or empty clears the rule's tags — the upsert rewrites the whole
   * row, so a partial edit has to send the existing list back. */
  tags?: string[];
  res_status: number;
  res_headers: RuleHeaderDto[];
  res_body_id?: string | null;
  res_body_base64?: string | null;
  res_body_mime?: string | null;
  res_delay_ms: number;
}

/** Which rules a bulk operation covers. `ids` is what a filtered view needs:
 *  the visible subset is arbitrary, and the button must never act on rows the
 *  user cannot see. */
export type RuleBulkScope =
  | { kind: "all" }
  | { kind: "ungrouped" }
  | { kind: "collection"; id: string }
  | { kind: "ids"; ids: string[] };

export interface RuleCollectionDto {
  id: string;
  name: string;
  enabled: boolean;
  priority: number;
  /** Same as `RuleDto.tags`. A collection's tags also match its rules in the
   * filter, so labelling a group labels everything in it. */
  tags: string[];
  rule_count: number;
  created_at: string;
  updated_at: string;
}

export interface CollectionUpsertArgs {
  id?: string | null;
  name: string;
  enabled: boolean;
  priority: number;
  tags?: string[];
}

/** One host the proxy decided not to decrypt, with the evidence for it. */
export interface TunneledHostDto {
  host: string;
  learned_at: string;
  /** "cert_rejected" (client sent a TLS alert) or "repeated_failure". */
  reason: string;
  detail: string;
}

export interface TunneledHostsDto {
  /** Learned this proxy run; individually forgettable and cleared on reset. */
  learned: TunneledHostDto[];
  /** Baked-in app-pinning patterns. Always tunnelled, never clearable. */
  seeded: string[];
}

/** Whether the device trusts our CA at all, for the current session. */
export interface TlsHealthDto {
  /** Distinct hosts tunnelled without decryption in this session. */
  tunneled_hosts: number;
  /** HTTPS captures successfully decrypted in this session. */
  decrypted_https: number;
}
