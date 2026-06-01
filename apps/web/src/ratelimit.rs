//! In-memory rate limiter — fixed window, per-IP.
//!
//! For 50 alpha users a global Mutex is fine; we don't need DashMap or
//! a token bucket. If the limiter ever becomes a hotspot the fix is to
//! move to `tower-governor`, but right now we don't pay the dep cost.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct RateLimiter {
    window: Duration,
    max_per_window: usize,
    state: Mutex<HashMap<IpAddr, Vec<Instant>>>,
}

impl RateLimiter {
    /// `max_per_minute` requests per IP, rolling 60-second window.
    #[must_use]
    pub fn per_minute(max_per_minute: usize) -> Self {
        Self {
            window: Duration::from_secs(60),
            max_per_window: max_per_minute,
            state: Mutex::new(HashMap::new()),
        }
    }

    /// `true` if the IP is within budget for *this* request. Records
    /// the hit when allowed. Idempotent across simultaneous calls
    /// because the mutex serialises bookkeeping.
    pub fn check(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let mut guard = self.state.lock().expect("rate limiter poisoned");
        let cutoff = now - self.window;
        let hits = guard.entry(ip).or_default();
        hits.retain(|t| *t > cutoff);
        if hits.len() >= self.max_per_window {
            return false;
        }
        hits.push(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn allows_up_to_max_then_blocks() {
        let rl = RateLimiter {
            window: Duration::from_secs(60),
            max_per_window: 3,
            state: Mutex::new(HashMap::new()),
        };
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        assert!(rl.check(ip));
        assert!(rl.check(ip));
        assert!(rl.check(ip));
        assert!(!rl.check(ip));
    }

    #[test]
    fn separate_ips_have_separate_budgets() {
        let rl = RateLimiter {
            window: Duration::from_secs(60),
            max_per_window: 1,
            state: Mutex::new(HashMap::new()),
        };
        let a = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let b = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        assert!(rl.check(a));
        assert!(rl.check(b));
        assert!(!rl.check(a));
        assert!(!rl.check(b));
    }
}
