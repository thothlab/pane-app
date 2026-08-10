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

/// What `assign` handed out, and what the caller needs to know about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortAssignment {
    /// The Mac-side port this device's `adb reverse` should target.
    pub port: u16,
    /// False when the pool was exhausted and this device had to share a port
    /// with another one. Traffic on a shared port can't be traced to either
    /// device, so it is deliberately left unattributed — see `assign`.
    pub attributed: bool,
    /// True when this call reserved the port, rather than returning one the
    /// serial already held. Lets a caller whose setup then failed give back
    /// exactly what it took and nothing else.
    pub fresh: bool,
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
    /// Pool exhaustion (9th+ device) falls back to the first pool port so the
    /// device still gets a working proxy. What it does *not* do is keep a
    /// device_id on that port: two devices reversing onto one local port means
    /// a connection arriving there could have come from either, and the old
    /// behaviour — last writer wins — silently stamped every one of device #1's
    /// captures with device #9's id. Unattributed is wrong-in-a-visible-way
    /// ("—" in the Devices column); a confident wrong label is not.
    ///
    /// The pool caps at 8 by design, so this is a safety net rather than an
    /// expected path — and now that failed pairings hand their port back, it
    /// takes eight genuinely-paired devices to reach it.
    pub fn assign(&self, serial: &str, device_id: Option<String>) -> PortAssignment {
        let mut inner = self.inner.lock();
        let (port, fresh) = match inner.serial_to_port.get(serial) {
            Some(&p) => (p, false),
            None => {
                let used: std::collections::HashSet<u16> =
                    inner.serial_to_port.values().copied().collect();
                let port = PROXY_PORT_POOL
                    .iter()
                    .copied()
                    .find(|p| !used.contains(p))
                    .unwrap_or(PROXY_PORT_POOL[0]);
                inner.serial_to_port.insert(serial.to_string(), port);
                (port, true)
            }
        };
        let shared = inner
            .serial_to_port
            .values()
            .filter(|&&p| p == port)
            .count()
            > 1;
        if shared {
            inner.port_to_device.remove(&port);
        } else if let Some(id) = device_id {
            inner.port_to_device.insert(port, id);
        }
        PortAssignment {
            port,
            attributed: !shared,
            fresh,
        }
    }

    /// Free the port held by `serial` (device removal). The `port → device_id`
    /// mapping is dropped too, so any late connection on that port resolves to
    /// NULL rather than mis-attributing to a now-gone device.
    ///
    /// Only call this once the device's `adb reverse` is actually gone. A port
    /// released while the phone still reverses onto it goes back in the pool,
    /// gets handed to the next device, and then that device's id is stamped on
    /// the first phone's traffic — see `DeviceManager::remove`, which keeps the
    /// reservation when teardown failed.
    ///
    /// If the port was shared (pool exhaustion), the remaining holder becomes
    /// its sole owner and is re-attributed by the next `assign` for that
    /// serial — the watchdog issues one every few seconds when it probes.
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
        assert_eq!(p1.port, p2.port);
        assert_eq!(
            p1.port, 8891,
            "first device gets the first pool port (8888 reserved)"
        );
        assert!(p1.fresh, "first assign reserved the port");
        assert!(!p2.fresh, "second assign only reported the existing one");
        assert!(p1.attributed && p2.attributed);
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
            a.port, 8888,
            "device must not be attributed to the Mac-local port"
        );
    }

    #[test]
    fn distinct_serials_get_distinct_ports() {
        let r = DevicePortRegistry::new();
        let a = r.assign("A", None);
        let b = r.assign("B", None);
        assert_ne!(a.port, b.port);
        assert_eq!(a.port, 8891);
        assert_eq!(b.port, 8892);
    }

    #[test]
    fn release_frees_the_port_for_reuse() {
        let r = DevicePortRegistry::new();
        let a = r.assign("A", Some("dev-a".into()));
        r.release("A");
        assert_eq!(r.device_for_port(a.port), None);
        // The freed port is the lowest free one again.
        let c = r.assign("C", Some("dev-c".into()));
        assert_eq!(c.port, a.port);
        assert_eq!(r.device_for_port(c.port).as_deref(), Some("dev-c"));
    }

    #[test]
    fn an_exhausted_pool_shares_a_port_without_faking_attribution() {
        // Nine devices, eight ports. The ninth shares 8891 with the first —
        // that part is unavoidable and it still gets a working proxy. What must
        // NOT happen is the old behaviour: the last assign overwriting the
        // mapping so every one of device #1's captures was stamped with the
        // other device's id.
        let r = DevicePortRegistry::new();
        for i in 0..PROXY_PORT_POOL.len() {
            let a = r.assign(&format!("S{i}"), Some(format!("dev-{i}")));
            assert!(a.attributed, "device {i} fits in the pool");
        }
        assert_eq!(
            r.device_for_port(PROXY_PORT_POOL[0]).as_deref(),
            Some("dev-0")
        );

        let overflow = r.assign("S8", Some("dev-8".into()));
        assert_eq!(
            overflow.port, PROXY_PORT_POOL[0],
            "falls back to the first port"
        );
        assert!(
            !overflow.attributed,
            "a shared port can't attribute anything"
        );
        assert_eq!(
            r.device_for_port(PROXY_PORT_POOL[0]),
            None,
            "neither device is labelled, rather than one wearing the other's id"
        );
    }

    #[test]
    fn freeing_a_shared_port_lets_the_survivor_be_attributed_again() {
        let r = DevicePortRegistry::new();
        for i in 0..=PROXY_PORT_POOL.len() {
            r.assign(&format!("S{i}"), Some(format!("dev-{i}")));
        }
        r.release("S8");
        // Re-attribution happens on the next assign, which the watchdog issues
        // every few seconds via probe_android_proxy.
        let again = r.assign("S0", Some("dev-0".into()));
        assert!(again.attributed);
        assert!(!again.fresh, "S0 kept the port it already held");
        assert_eq!(
            r.device_for_port(PROXY_PORT_POOL[0]).as_deref(),
            Some("dev-0")
        );
    }

    #[test]
    fn fresh_marks_only_the_call_that_reserved_the_port() {
        // Drives the rollback in add_android_usb: a failed pairing must return
        // the port it just took, and must NOT return one an earlier successful
        // pairing is still using.
        let r = DevicePortRegistry::new();
        assert!(r.assign("A", None).fresh);
        assert!(!r.assign("A", None).fresh);
        r.release("A");
        assert!(r.assign("A", None).fresh, "after release it is fresh again");
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
        assert_eq!(r.device_for_port(p.port).as_deref(), Some("dev-a"));
        assert_eq!(r.device_for_port(9999), None);
    }
}
