use super::{to_api, CmdResult};
use crate::state::AppState;
use pane_engine::{EngineConfig, ProxyEngine};
use pane_engine_mitm::MitmEngine;
use pane_ipc::{ProxyStartArgs, SessionDto, ProxyStatusDto};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub async fn start(
    app: AppHandle,
    state: State<'_, AppState>,
    args: ProxyStartArgs,
) -> CmdResult<SessionDto> {
    let host = args.host.unwrap_or_else(|| "127.0.0.1".into());
    let port = args.port.unwrap_or(8888);
    let listen = format!("{host}:{port}")
        .parse()
        .map_err(to_api("invalid_addr"))?;

    // PAC sits on the same host one port up. The Android `http_proxy_pac`
    // setting points at it (via adb reverse); when Pane goes away the
    // device falls back to DIRECT instead of stranding on a dead proxy.
    let pac_listen: std::net::SocketAddr = format!("{host}:{}", port + 1)
        .parse()
        .map_err(to_api("invalid_addr"))?;

    let ca_material = state.ca.material();
    let engine: Arc<dyn ProxyEngine> = Arc::new(MitmEngine::new(state.storage.clone()));
    let handle = engine
        .start(EngineConfig {
            listen,
            ca: ca_material,
            pac_listen: Some(pac_listen),
        })
        .await
        .map_err(to_api("engine_start"))?;

    // Forward engine events to the UI bus.
    let mut rx = engine.events();
    let app_clone = app.clone();
    tokio::spawn(async move {
        while let Ok(ev) = rx.recv().await {
            let _ = app_clone.emit(ev.topic(), ev.payload());
        }
    });

    let session = state.storage.session_record(listen).map_err(to_api("db"))?;
    *state.proxy_handle.lock() = Some(handle);
    let _ = app.emit("proxy.status_changed", &session);
    Ok(session)
}

#[tauri::command]
pub async fn stop(state: State<'_, AppState>) -> CmdResult<serde_json::Value> {
    let handle = state.proxy_handle.lock().take();
    if let Some(h) = handle {
        h.shutdown().await.map_err(to_api("engine_stop"))?;
    }
    // Clear http_proxy + adb-reverse on every paired Android device.
    // Otherwise the phone keeps pointing at 127.0.0.1:8888 which now
    // refuses connections — manifesting on the device as "no internet"
    // until the user notices and removes the device manually.
    let cleared = state.devices.clear_all_android_proxies().await;
    Ok(serde_json::json!({
        "stopped_at": time::OffsetDateTime::now_utc().to_string(),
        "cleared_devices": cleared,
    }))
}

#[tauri::command]
pub async fn status(state: State<'_, AppState>) -> CmdResult<ProxyStatusDto> {
    let running = state.proxy_handle.lock().is_some();
    let count = state.storage.captures_count().map_err(to_api("db"))? as u64;
    Ok(ProxyStatusDto {
        running,
        captures_count: count,
        listen: state
            .proxy_handle
            .lock()
            .as_ref()
            .map(|h| h.listen.to_string()),
    })
}
