//! Per-device proxy-port registry.
//!
//! Every USB Android device routes through `adb reverse tcp:8888 tcp:<host>`,
//! so on the Mac side each device's connections arrive on a *different* local
//! port even though the device always talks to its own `127.0.0.1:8888`. This
//! registry maps `serial → host_port` (for the `adb reverse` setup) and
//! `host_port → device_id` (for the proxy accept loop to stamp captures).
//!
//! Shared between the Android platform wiring (which assigns a port per serial
//! when pairing) and the proxy accept path (which resolves device_id from the
//! local port a connection landed on). Mutex-guarded so the known
//! concurrent-`add_usb`-on-same-serial race can't hand out two ports.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

/// Mac-side data-proxy port pool for ATTRIBUTED (per-device) traffic.
///
/// `8888` is deliberately NOT in the pool: it's the canonical proxy port the
/// engine always binds, and it carries direct Mac-local traffic (e.g. a
/// desktop browser pointed at `127.0.0.1:8888`). Leaving it unassigned means
/// such traffic resolves to `device_id = NULL` ("—") instead of being
/// mislabelled as whichever device happened to grab 8888 first. Each USB
/// device instead reverses `tcp:8888 tcp:<pool port>` so its traffic lands on
/// a distinct port we can attribute. `8889`/`8890` are the shared PAC and
/// heartbeat services. Eight devices is well past any realistic desk setup.
pub const PROXY_PORT_POOL: [u16; 8] = [8891, 8892, 8893, 8894, 8895, 8896, 8897, 8898];

#[derive(Default)]
struct Inner {
    /// serial → assigned host port.
    serial_to_port: HashMap<String, u16>,
    /// host port → device_id (persisted device-row id). `None` device_id is
    /// never stored — unmapped ports simply have no entry, so a connection on
    /// such a port resolves to `None` and the capture's device_id is NULL.
    port_to_device: HashMap<u16, String>,
}

/// Thread-safe, cheaply cloneable handle to the shared port registry.
#[derive(Clone, Default)]
pub struct DevicePortRegistry {
    inner: Arc<Mutex<Inner>>,
}

impl DevicePortRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Idempotent per serial: returns the SAME port if this serial already has
    /// one (re-pair / reapply / concurrent add). On first assignment, picks the
    /// lowest free port from the pool. `device_id` is the persisted device-row
    /// id used to stamp captures; passing `None` (e.g. id not yet resolvable)
    /// still reserves the port but leaves it unattributed until a later assign
    /// supplies the id.
    ///
    /// Pool exhaustion (9th+ device) falls back to the first pool port — that
    /// device shares a port (and thus attribution) with device #1, but still
    /// gets a working proxy. The pool caps at 8 by design, so this is a safety
    /// net, not an expected path.
    pub fn assign(&self, serial: &str, device_id: Option<String>) -> u16 {
        let mut inner = self.inner.lock();
        let port = match inner.serial_to_port.get(serial) {
            Some(&p) => p,
            None => {
                let used: std::collections::HashSet<u16> =
                    inner.serial_to_port.values().copied().collect();
                let port = PROXY_PORT_POOL
                    .iter()
                    .copied()
                    .find(|p| !used.contains(p))
                    .unwrap_or(PROXY_PORT_POOL[0]);
                inner.serial_to_port.insert(serial.to_string(), port);
                port
            }
        };
        if let Some(id) = device_id {
            inner.port_to_device.insert(port, id);
        }
        port
    }

    /// Free the port held by `serial` (device removal). The `port → device_id`
    /// mapping is dropped too, so any late connection on that port resolves to
    /// NULL rather than mis-attributing to a now-gone device.
    pub fn release(&self, serial: &str) {
        let mut inner = self.inner.lock();
        if let Some(port) = inner.serial_to_port.remove(serial) {
            inner.port_to_device.remove(&port);
        }
    }

    /// Resolve the persisted device_id for a connection that arrived on
    /// `host_port`. `None` for ports with no current mapping (old captures,
    /// iOS, unattributed) → capture device_id stays NULL.
    pub fn device_for_port(&self, host_port: u16) -> Option<String> {
        self.inner.lock().port_to_device.get(&host_port).cloned()
    }

    /// Stamp an explicit `port → device_id` mapping, bypassing the serial pool.
    /// Used for the reserved host port 8888 → "__host__" sentinel when the user
    /// captures their own Mac: connections on 8888 then resolve to the host
    /// sentinel via `device_for_port` with no change to the accept loop.
    pub fn set_port(&self, port: u16, device_id: &str) {
        self.inner
            .lock()
            .port_to_device
            .insert(port, device_id.to_string());
    }

    /// Drop an explicit port mapping set by `set_port` (host capture disabled).
    /// After this, a connection on `port` resolves to `None` again.
    pub fn clear_port(&self, port: u16) {
        self.inner.lock().port_to_device.remove(&port);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assign_is_idempotent_per_serial() {
        let r = DevicePortRegistry::new();
        let p1 = r.assign("A", Some("dev-a".into()));
        let p2 = r.assign("A", Some("dev-a".into()));
        assert_eq!(p1, p2);
        assert_eq!(
            p1, 8891,
            "first device gets the first pool port (8888 reserved)"
        );
    }

    #[test]
    fn devices_never_get_the_reserved_8888() {
        let r = DevicePortRegistry::new();
        assert!(
            !PROXY_PORT_POOL.contains(&8888),
            "8888 must stay out of the pool"
        );
        let a = r.assign("A", None);
        assert_ne!(
            a, 8888,
            "device must not be attributed to the Mac-local port"
        );
    }

    #[test]
    fn distinct_serials_get_distinct_ports() {
        let r = DevicePortRegistry::new();
        let a = r.assign("A", None);
        let b = r.assign("B", None);
        assert_ne!(a, b);
        assert_eq!(a, 8891);
        assert_eq!(b, 8892);
    }

    #[test]
    fn release_frees_the_port_for_reuse() {
        let r = DevicePortRegistry::new();
        let a = r.assign("A", Some("dev-a".into()));
        r.release("A");
        assert_eq!(r.device_for_port(a), None);
        // The freed port is the lowest free one again.
        let c = r.assign("C", Some("dev-c".into()));
        assert_eq!(c, a);
        assert_eq!(r.device_for_port(c).as_deref(), Some("dev-c"));
    }

    #[test]
    fn set_port_and_clear_port_for_host_sentinel() {
        let r = DevicePortRegistry::new();
        // 8888 is reserved (not in the pool) and unmapped by default.
        assert_eq!(r.device_for_port(8888), None);
        r.set_port(8888, "__host__");
        assert_eq!(r.device_for_port(8888).as_deref(), Some("__host__"));
        r.clear_port(8888);
        assert_eq!(r.device_for_port(8888), None);
    }

    #[test]
    fn port_resolves_to_device_id() {
        let r = DevicePortRegistry::new();
        let p = r.assign("A", Some("dev-a".into()));
        assert_eq!(r.device_for_port(p).as_deref(), Some("dev-a"));
        assert_eq!(r.device_for_port(9999), None);
    }
}
