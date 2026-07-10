// Thin invoke wrapper. Centralizes argument shape so call sites stay typed.

import { invoke } from "@tauri-apps/api/core";
import type {
  ApiError,
  CaCertificateDto,
  CaExportResult,
  CaSaveResult,
  CaptureBodyDto,
  CaptureDto,
  AndroidToolingStatusDto,
  DeviceDto,
  DiscoveredDeviceDto,
  FilterDto,
  LogcatRowDto,
  ProxyStatusDto,
  CollectionUpsertArgs,
  ReplayRecordDto,
  RequestSpec,
  RuleCollectionDto,
  RuleDto,
  RuleUpsertArgs,
  SessionDto,
} from "./types";

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return (await invoke<T>(cmd, args ?? {})) as T;
  } catch (e) {
    const err = e as ApiError;
    console.error(`ipc:${cmd} failed`, err);
    throw err;
  }
}

export const api = {
  proxy: {
    start: (host?: string, port?: number) =>
      call<SessionDto>("start", { args: { host, port } }),
    stop: () => call<{ stopped_at: string }>("stop"),
    status: () => call<ProxyStatusDto>("status"),
  },
  host: {
    // "Capture this Mac": trust the CA + point the Mac's system proxy at the
    // local MITM port. enable/status return { enabled, service }. macOS-only;
    // the backend command errors on other platforms.
    enable: () =>
      call<{ enabled: boolean; service: string | null }>("host_capture_enable"),
    disable: () => call<void>("host_capture_disable"),
    status: () =>
      call<{ enabled: boolean; service: string | null }>("host_capture_status"),
  },
  ca: {
    current: () => call<CaCertificateDto>("current"),
    rotate: () => call<CaCertificateDto>("rotate"),
    export: (format: "pem" | "der" | "qr" | "mobileconfig") =>
      call<CaExportResult>("export", { args: { format } }),
    saveToFile: (format: "pem" | "der" | "qr" | "mobileconfig", path: string) =>
      call<CaSaveResult>("save_to_file", { args: { format, path } }),
  },
  devices: {
    listAttachedUsb: () => call<DiscoveredDeviceDto[]>("list_attached_usb"),
    addIosUsb: (serial: string) =>
      call<DeviceDto>("add_ios_usb", { args: { serial } }),
    addAndroidUsb: (serial: string) =>
      call<DeviceDto>("add_android_usb", { args: { serial } }),
    remove: (id: string) =>
      call<{ cleaned: boolean; pending_cleanup: boolean }>("remove", { args: { id } }),
    get: (id: string) => call<DeviceDto>("devices_get", { id }),
    list: () => call<DeviceDto[]>("devices_list"),
    androidToolingStatus: () =>
      call<AndroidToolingStatusDto>("android_tooling_status"),
  },
  captures: {
    list: (filter?: string, limit = 500, before?: string) =>
      call<CaptureDto[]>("captures_list", { args: { filter, limit, before } }),
    get: (id: string) => call<CaptureDto>("captures_get", { id }),
    body: (bodyId: string, maxBytes?: number) =>
      call<CaptureBodyDto>("get_body", { args: { body_id: bodyId, max_bytes: maxBytes } }),
    clear: (olderThan?: string) =>
      call<{ deleted: number }>("clear", { args: { older_than: olderThan } }),
    exportOne: (id: string, format: "curl" | "har_single") =>
      call<{ text: string; mime: string }>("export_one", { args: { id, format } }),
  },
  replay: {
    send: (request: RequestSpec, sourceId?: string) =>
      call<ReplayRecordDto>("send", { args: { source_id: sourceId, request } }),
  },
  filters: {
    save: (f: {
      id?: string;
      name: string;
      query: string;
      color: string;
      pinned: boolean;
      kind: "captures" | "logcat";
    }) => call<FilterDto>("filters_save", { args: f }),
    list: (kind?: "captures" | "logcat") =>
      call<FilterDto[]>("filters_list", kind ? { kind } : {}),
    delete: (id: string) => call<{ deleted: true }>("filters_delete", { id }),
  },
  rules: {
    list: () => call<RuleDto[]>("rules_list"),
    get: (id: string) => call<RuleDto>("rule_get", { id }),
    upsert: (args: RuleUpsertArgs) => call<RuleDto>("rule_upsert", { args }),
    delete: (id: string) => call<void>("rule_delete", { id }),
    setEnabled: (id: string, enabled: boolean) =>
      call<void>("rule_set_enabled", { args: { id, enabled } }),
    setPriority: (id: string, priority: number) =>
      call<void>("rule_set_priority", { args: { id, priority } }),
    exportWrite: (path: string, content: string) =>
      call<number>("rules_export_write", { path, content }),
    importRead: (path: string) => call<string>("rules_import_read", { path }),
  },
  collections: {
    list: () => call<RuleCollectionDto[]>("collections_list"),
    upsert: (args: CollectionUpsertArgs) =>
      call<RuleCollectionDto>("collection_upsert", { args }),
    delete: (id: string) => call<void>("collection_delete", { id }),
    setEnabled: (id: string, enabled: boolean) =>
      call<void>("collection_set_enabled", { args: { id, enabled } }),
    setPriority: (id: string, priority: number) =>
      call<void>("collection_set_priority", { args: { id, priority } }),
  },
  logcat: {
    // `filter` is the raw DSL string; app: PIDs are resolved on the client and
    // passed as include/exclude lists. Returns rows oldest→newest.
    query: (
      serial: string,
      filter?: string,
      includePids: number[] = [],
      excludePids: number[] = [],
      limit = 2000,
    ) =>
      call<LogcatRowDto[]>("logcat_query", {
        args: {
          serial,
          filter,
          include_pids: includePids,
          exclude_pids: excludePids,
          limit,
        },
      }),
    // "Load older on scroll-up": newest `limit` rows older than `beforeId`.
    // Returns rows oldest→newest, ready to prepend to the loaded window.
    queryOlder: (
      serial: string,
      filter: string | undefined,
      includePids: number[],
      excludePids: number[],
      beforeId: number,
      limit: number,
    ) =>
      call<LogcatRowDto[]>("logcat_query_older", {
        args: {
          serial,
          filter,
          include_pids: includePids,
          exclude_pids: excludePids,
          before_id: beforeId,
          limit,
        },
      }),
    newCount: (
      serial: string,
      filter: string | undefined,
      includePids: number[],
      excludePids: number[],
      afterId: number,
    ) =>
      call<number>("logcat_new_count", {
        args: {
          serial,
          filter,
          include_pids: includePids,
          exclude_pids: excludePids,
          after_id: afterId,
        },
      }),
    clear: (serial: string) =>
      call<{ deleted: number }>("logcat_clear", { args: { serial } }),
    export: (
      serial: string,
      filter: string | undefined,
      includePids: number[],
      excludePids: number[],
      path: string,
    ) =>
      call<number>("logcat_export", {
        args: {
          serial,
          filter,
          include_pids: includePids,
          exclude_pids: excludePids,
          path,
        },
      }),
  },
};
