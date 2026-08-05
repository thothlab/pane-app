use std::sync::Arc;

use pane_engine::{EngineConfig, ProxyEngine};
use pane_engine_mitm::MitmEngine;
use pane_ipc::{kinds, ProxyStartArgs, ProxyStatusDto, SessionDto};

use crate::error::{to_api, CoreResult};
use crate::events::{topics, CoreEvent};
use crate::Core;

impl Core {
    /// Start the MITM proxy.
    ///
    /// Unlike the old command this takes no `AppHandle`: engine events are
    /// pumped onto [`Core::events`], and whoever wants them subscribes.
    pub async fn proxy_start(&self, args: ProxyStartArgs) -> CoreResult<SessionDto> {
        let host = args.host.unwrap_or_else(|| "127.0.0.1".into());
        let port = args.port.unwrap_or(8888);
        let listen = format!("{host}:{port}")
            .parse()
            .map_err(to_api(kinds::INVALID_ADDR))?;

        // PAC sits on the same host one port up. The Android `http_proxy_pac`
        // setting points at it (via adb reverse); when Pane goes away the
        // device falls back to DIRECT instead of stranding on a dead proxy.
        let pac_listen: std::net::SocketAddr = format!("{host}:{}", port + 1)
            .parse()
            .map_err(to_api(kinds::INVALID_ADDR))?;

        // Heartbeat lives two ports up from the MITM port. The companion APK
        // on each paired Android device connects to this (adb-reverse-
        // forwarded) and pings every 2s. When it loses the connection (USB
        // unplug, Pane quit) it clears the device's http_proxy so the user
        // doesn't get stranded with no internet.
        let heartbeat_listen: std::net::SocketAddr = format!("{host}:{}", port + 2)
            .parse()
            .map_err(to_api(kinds::INVALID_ADDR))?;

        let ca_material = self.ca.material();
        let engine: Arc<dyn ProxyEngine> = Arc::new(MitmEngine::new(self.storage.clone()));
        let handle = engine
            .start(EngineConfig {
                listen,
                ca: ca_material,
                pac_listen: Some(pac_listen),
                heartbeat_listen: Some(heartbeat_listen),
                registry: self.registry.clone(),
            })
            .await
            .map_err(to_api(kinds::ENGINE_START))?;

        // Pump engine events onto the core bus. Retaining the `Arc<dyn
        // ProxyEngine>` below is what keeps the engine's own broadcast sender
        // alive; without it this receiver would be the only one and no later
        // subscriber could ever be created.
        let mut rx = engine.events();
        let bus = self.events.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ev) => bus.publish(CoreEvent::new(ev.topic(), ev.payload())),
                    // Lagged means this pump fell behind the engine; keep
                    // going rather than tearing the bus down.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "engine event pump lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        let session = self
            .storage
            .session_record(listen)
            .map_err(to_api(kinds::DB))?;
        *self.proxy_handle.lock() = Some(handle);
        *self.engine.lock() = Some(engine);

        self.events.publish_topic(
            topics::PROXY_STATUS_CHANGED,
            serde_json::to_value(&session).unwrap_or(serde_json::Value::Null),
        );

        // Re-apply PAC + adb reverse on every paired Android. Without this, a
        // Stop → Start cycle (or any time Pane was closed while devices were
        // paired) leaves the phone with cleared proxy settings and no reverse
        // — traffic never reaches Pane until the user clicks Re-sync on each
        // device row by hand.
        let ca_for_reapply = self.ca.material();
        let devices = self.devices.clone();
        tokio::spawn(async move {
            let reapplied = devices.reapply_all_android_proxies(ca_for_reapply).await;
            if !reapplied.is_empty() {
                tracing::info!(devices = ?reapplied, "auto-reapplied proxy on paired Android");
            }
        });

        Ok(session)
    }

    /// Stop the proxy and undo everything that pointed at it.
    pub async fn proxy_stop(&self) -> CoreResult<serde_json::Value> {
        let handle = self.proxy_handle.lock().take();
        if let Some(h) = handle {
            h.shutdown().await.map_err(to_api(kinds::ENGINE_STOP))?;
        }
        // Drop the engine now that it's shut down, so the next start gets a
        // fresh one.
        self.engine.lock().take();

        // Clear http_proxy + adb-reverse on every paired Android device.
        // Otherwise the phone keeps pointing at 127.0.0.1:8888 which now
        // refuses connections — manifesting on the device as "no internet".
        let cleared = self.devices.clear_all_android_proxies().await;

        // Revert the Mac's own system proxy too, if "Capture this Mac" was on.
        if let Err(e) = crate::host_proxy::disable(self) {
            tracing::warn!(error = %e, "failed to revert host proxy on stop");
        }

        self.events.publish_topic(
            topics::PROXY_STATUS_CHANGED,
            serde_json::json!({ "running": false }),
        );

        Ok(serde_json::json!({
            "stopped_at": time::OffsetDateTime::now_utc().to_string(),
            "cleared_devices": cleared,
        }))
    }

    pub async fn proxy_status(&self) -> CoreResult<ProxyStatusDto> {
        let count = self.storage.captures_count().map_err(to_api(kinds::DB))? as u64;
        Ok(ProxyStatusDto {
            running: self.proxy_running(),
            captures_count: count,
            listen: self.proxy_listen(),
        })
    }
}
