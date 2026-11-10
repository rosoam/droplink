//! Rate limiting for session attempts.
//!
//! Two budgets: one per network prefix (IPv4 /32, IPv6 /64) and one global. The /64 grouping is
//! not cosmetic: a single IPv6 host controls 2^64 addresses and would otherwise bypass any
//! per-IP limit.

use droplink_core::relay::{
    SESSION_GLOBAL_ATTEMPTS_PER_SEC, SESSION_JOIN_ATTEMPTS_PER_MIN, SESSION_JOIN_BURST,
};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::{Duration, Instant};

/// Network identity used for the per-prefix budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NetKey {
    addr: IpAddr,
    prefix: u8,
}

impl NetKey {
    pub fn of(ip: IpAddr) -> Self {
        match ip {
            IpAddr::V4(v4) => Self {
                addr: IpAddr::V4(v4),
                prefix: 32,
            },
            IpAddr::V6(v6) => {
                let segments = v6.segments();
                Self {
                    addr: IpAddr::V6(Ipv6Addr::new(
                        segments[0],
                        segments[1],
                        segments[2],
                        segments[3],
                        0,
                        0,
                        0,
                        0,
                    )),
                    prefix: 64,
                }
            }
        }
    }

    pub fn is_ipv4_loopback(&self) -> bool {
        matches!(self.addr, IpAddr::V4(v4) if v4 == Ipv4Addr::LOCALHOST)
    }
}

struct Bucket {
    tokens: f64,
    capacity: f64,
    rate_per_sec: f64,
    last: Instant,
    touched: Instant,
}

impl Bucket {
    fn new(capacity: f64, rate_per_sec: f64) -> Self {
        Self {
            tokens: capacity,
            capacity,
            rate_per_sec,
            last: Instant::now(),
            touched: Instant::now(),
        }
    }

    fn allow(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.last = now;
        self.touched = now;
        self.tokens = (self.tokens + elapsed * self.rate_per_sec).min(self.capacity);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Per-network and global budgets for `CreateSession` / `JoinSession`.
pub struct SessionLimiter {
    per_net: HashMap<NetKey, Bucket>,
    global: Bucket,
}

impl Default for SessionLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionLimiter {
    pub fn new() -> Self {
        Self {
            per_net: HashMap::new(),
            global: Bucket::new(
                SESSION_GLOBAL_ATTEMPTS_PER_SEC as f64,
                SESSION_GLOBAL_ATTEMPTS_PER_SEC as f64,
            ),
        }
    }

    /// Reserve one attempt. Both budgets must allow it.
    pub fn allow(&mut self, ip: IpAddr) -> bool {
        self.purge();
        if !self.global.allow() {
            return false;
        }
        let key = NetKey::of(ip);
        let bucket = self.per_net.entry(key).or_insert_with(|| {
            Bucket::new(
                SESSION_JOIN_BURST as f64,
                SESSION_JOIN_ATTEMPTS_PER_MIN as f64 / 60.0,
            )
        });
        bucket.allow()
    }

    fn purge(&mut self) {
        if self.per_net.len() < 4096 {
            return;
        }
        let cutoff = Instant::now() - Duration::from_secs(600);
        self.per_net.retain(|_, bucket| bucket.touched > cutoff);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_then_refusal() {
        let mut limiter = SessionLimiter::new();
        let ip: IpAddr = "203.0.113.5".parse().unwrap();
        for _ in 0..SESSION_JOIN_BURST {
            assert!(limiter.allow(ip));
        }
        assert!(!limiter.allow(ip));
    }

    #[test]
    fn another_network_is_unaffected() {
        let mut limiter = SessionLimiter::new();
        let a: IpAddr = "203.0.113.5".parse().unwrap();
        let b: IpAddr = "198.51.100.5".parse().unwrap();
        for _ in 0..SESSION_JOIN_BURST {
            assert!(limiter.allow(a));
        }
        assert!(!limiter.allow(a));
        assert!(limiter.allow(b));
    }

    #[test]
    fn ipv6_is_grouped_by_64() {
        let mut limiter = SessionLimiter::new();
        let a: IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2:ffff:ffff:ffff:ffff".parse().unwrap();
        let other: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        for _ in 0..SESSION_JOIN_BURST {
            assert!(limiter.allow(a));
        }
        assert!(!limiter.allow(b));
        assert!(limiter.allow(other));
    }

    #[test]
    fn one_hundred_attempts_from_one_64_allow_only_the_burst() {
        let mut limiter = SessionLimiter::new();
        let ip: IpAddr = "2001:db8:1:2::abcd".parse().unwrap();
        let allowed = (0..100).filter(|_| limiter.allow(ip)).count();
        assert_eq!(
            allowed as u32, SESSION_JOIN_BURST,
            "a /64 must not get more than the burst"
        );
        // The global budget may also be exhausted by a 100-attempt burst; that is by design.
        // Per-network isolation without a flood is covered by `another_network_is_unaffected`.
    }

    #[test]
    fn net_key_normalises_prefixes() {
        let a: IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2::2".parse().unwrap();
        assert_eq!(NetKey::of(a), NetKey::of(b));
        assert_ne!(
            NetKey::of(a),
            NetKey::of("2001:db8:1:3::1".parse().unwrap())
        );
    }
}
