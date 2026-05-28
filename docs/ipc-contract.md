# IPC contract

Source of truth: `crates/mycharles-ipc/src/lib.rs`. TS mirror lives at
`src/ipc/types.ts` (hand-maintained for MVP; specta codegen tracked as a
follow-up).

## Commands (Tauri `invoke`)

| Command | Args | Returns | Notes |
| ------- | ---- | ------- | ----- |
| `start` | `ProxyStartArgs` | `SessionDto` | Boots the proxy and writes a session row. |
| `stop`  | — | `{ stopped_at }` | Tears down the engine and cleans the broadcast bus. |
| `status` | — | `ProxyStatusDto` | Polled by the layout footer every 2s. |
| `current` | — | `CaCertificateDto` | Active root CA. |
| `rotate` | — | `CaCertificateDto` | Marks old CA revoked, generates new. |
| `export` | `CaExportArgs` | `CaExportResult` | PEM / DER / QR / mobileconfig. |
| `list_attached_usb` | — | `DiscoveredDeviceDto[]` | Combines iOS + Android probes. |
| `add_ios_usb` | `AddDeviceArgs` | `DeviceDto` | Runs the iOS USB pipeline (libimobiledevice). |
| `add_android_usb` | `AddDeviceArgs` | `DeviceDto` | Runs the Android USB pipeline (adb). |
| `remove` | `RemoveDeviceArgs` | `RemoveDeviceResult` | Best-effort cleanup. |
| `get` (device) | `id: Uuid` | `DeviceDto` | |
| `list` (devices) | — | `DeviceDto[]` | |
| `list` (captures) | `ListCapturesArgs` | `CaptureDto[]` | Filter DSL applied server-side. |
| `get` (capture)  | `id: Uuid` | `CaptureDto` (+ headers) | |
| `get_body` | `GetBodyArgs` | `CaptureBodyDto` | Base64 + truncation flag. |
| `clear` (captures) | `ClearArgs` | `ClearResult` | |
| `export_one` | `ExportOneArgs` | `ExportOneResult` | cURL or single-entry HAR. |
| `send` (replay) | `ReplaySendArgs` | `ReplayRecordDto` | |
| `save` (filter) | `SaveFilterArgs` | `FilterDto` | |
| `list` (filters) | — | `FilterDto[]` | |
| `delete` (filter) | `id: Uuid` | `{ deleted: true }` | |

## Events (Tauri event bus)

| Topic | Payload |
| ----- | ------- |
| `capture.started` | `{ id, host, method, path, started_at }` |
| `capture.headers` | `{ id, status }` |
| `capture.completed` | `{ id, status, duration_ms, total_bytes }` |
| `capture.error` | `{ id, host, error_kind, message }` |
| `pinning.detected` | `{ id, host, alpn? }` |
| `proxy.status_changed` | `SessionDto` |

## Error envelope

```jsonc
{
  "kind": "stable_machine_readable_string",
  "message": "human readable",
  "details": null | object
}
```

UI surfaces `message` to users and may switch on `kind` for special-case copy
(e.g. `port_in_use`, `pairing_denied`, `tooling_missing`).
