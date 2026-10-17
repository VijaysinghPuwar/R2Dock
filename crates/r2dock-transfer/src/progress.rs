//! Byte counters and speed estimates (PLAN.md 12.4).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Shared byte counter for one job. Parts add their in-flight bytes here.
#[derive(Debug, Default)]
pub struct Counter {
    /// Bytes of parts or ranges already confirmed.
    committed: AtomicU64,
    /// Bytes in flight in the current attempt(s).
    in_flight: AtomicU64,
}

impl Counter {
    pub fn total(&self) -> u64 {
        self.committed.load(Ordering::Relaxed) + self.in_flight.load(Ordering::Relaxed)
    }

    pub fn set_committed(&self, bytes: u64) {
        self.committed.store(bytes, Ordering::Relaxed);
    }

    pub fn commit(&self, bytes: u64) {
        self.committed.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn add_in_flight(&self, bytes: u64) {
        self.in_flight.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn sub_in_flight(&self, bytes: u64) {
        // Saturating: never underflow if a body was rebuilt mid-way.
        let mut cur = self.in_flight.load(Ordering::Relaxed);
        loop {
            let next = cur.saturating_sub(bytes);
            match self.in_flight.compare_exchange_weak(
                cur,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(actual) => cur = actual,
            }
        }
    }

    pub fn reset_in_flight(&self) {
        self.in_flight.store(0, Ordering::Relaxed);
    }
}

/// Exponentially weighted speed over roughly five seconds.
#[derive(Debug)]
pub struct SpeedMeter {
    started: Instant,
    last_at: Instant,
    last_bytes: u64,
    ewma: Option<f64>,
}

/// Show an ETA only after this much data.
pub const ETA_WARMUP: Duration = Duration::from_secs(3);
const WINDOW_SECS: f64 = 5.0;

impl SpeedMeter {
    pub fn new(now: Instant, bytes: u64) -> Self {
        Self {
            started: now,
            last_at: now,
            last_bytes: bytes,
            ewma: None,
        }
    }

    /// Record a sample; returns (speed in bytes/s, eta in seconds) when known.
    pub fn sample(&mut self, now: Instant, bytes: u64, total: u64) -> (Option<f64>, Option<f64>) {
        let dt = now.saturating_duration_since(self.last_at).as_secs_f64();
        if dt > 0.0 {
            let delta = bytes.saturating_sub(self.last_bytes) as f64;
            let instant = delta / dt;
            let alpha = 1.0 - (-dt / WINDOW_SECS).exp();
            self.ewma = Some(match self.ewma {
                Some(prev) => prev + alpha * (instant - prev),
                None => instant,
            });
            self.last_at = now;
            self.last_bytes = bytes;
        }
        let warmed = now.saturating_duration_since(self.started) >= ETA_WARMUP;
        let speed = if warmed { self.ewma } else { None };
        let eta = match speed {
            Some(s) if s > 1.0 && total >= bytes => Some((total - bytes) as f64 / s),
            _ => None,
        };
        (speed, eta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_math() {
        let c = Counter::default();
        c.commit(10);
        c.add_in_flight(5);
        assert_eq!(c.total(), 15);
        c.sub_in_flight(50);
        assert_eq!(c.total(), 10);
        c.set_committed(3);
        assert_eq!(c.total(), 3);
    }

    #[test]
    fn speed_hidden_until_warm() {
        let t0 = Instant::now();
        let mut m = SpeedMeter::new(t0, 0);
        let (s, e) = m.sample(t0 + Duration::from_secs(1), 1000, 10_000);
        assert!(s.is_none() && e.is_none());
        let (s, e) = m.sample(t0 + Duration::from_secs(4), 4000, 10_000);
        let s = s.unwrap();
        assert!(s > 900.0 && s < 1100.0, "{s}");
        let e = e.unwrap();
        assert!(e > 5.0 && e < 7.0, "{e}");
    }
}
