//! Hybrid logical clock.
//!
//! An HLC value is `millis * 1000 + counter`. The counter absorbs events that
//! share a millisecond (or happen while the wall clock is behind), and simply
//! carries into the millisecond part when it passes 999. Values stay below
//! 2^53 until the year 2255, so the frontend can read them as plain numbers.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Largest amount a peer's clock may be ahead of ours.
pub const MAX_SKEW_MS: i64 = 2 * 60 * 1000;

const COUNTER_SPAN: i64 = 1000;

/// Source of wall-clock time in UTC milliseconds.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

/// Settable clock for tests and the simulation.
#[derive(Debug, Default)]
pub struct ManualClock(AtomicI64);

impl ManualClock {
    pub fn new(ms: i64) -> Self {
        Self(AtomicI64::new(ms))
    }

    pub fn set(&self, ms: i64) {
        self.0.store(ms, Ordering::SeqCst);
    }

    pub fn advance(&self, ms: i64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Hlc(pub i64);

impl Hlc {
    pub const ZERO: Hlc = Hlc(0);

    pub fn from_millis(ms: i64) -> Self {
        Hlc(ms * COUNTER_SPAN)
    }

    pub fn millis(self) -> i64 {
        self.0.div_euclid(COUNTER_SPAN)
    }

    pub fn counter(self) -> i64 {
        self.0.rem_euclid(COUNTER_SPAN)
    }
}

/// The clock state owned by the writer thread.
#[derive(Debug, Clone, Copy)]
pub struct HlcState {
    last: Hlc,
}

impl HlcState {
    pub fn new(last: Hlc) -> Self {
        Self { last }
    }

    pub fn last(&self) -> Hlc {
        self.last
    }

    /// Next timestamp for a local event. Always greater than anything issued
    /// or observed before, even if the wall clock went backwards.
    pub fn tick(&mut self, now_ms: i64) -> Hlc {
        let wall = Hlc::from_millis(now_ms);
        self.last = if wall > self.last {
            wall
        } else {
            Hlc(self.last.0 + 1)
        };
        self.last
    }

    /// Takes a remote timestamp into account so later local events sort after
    /// it. Rejects timestamps too far ahead of our wall clock.
    pub fn observe(&mut self, remote: Hlc, now_ms: i64) -> Result<(), i64> {
        let ahead = remote.millis() - now_ms;
        if ahead > MAX_SKEW_MS {
            return Err(ahead);
        }
        if remote > self.last {
            self.last = remote;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_follows_wall_clock() {
        let mut h = HlcState::new(Hlc::ZERO);
        assert_eq!(h.tick(5_000), Hlc::from_millis(5_000));
        assert_eq!(h.tick(6_000), Hlc::from_millis(6_000));
    }

    #[test]
    fn tick_is_monotonic_when_clock_stalls_or_goes_back() {
        let mut h = HlcState::new(Hlc::ZERO);
        let a = h.tick(10_000);
        let b = h.tick(10_000);
        let c = h.tick(9_000);
        assert!(a < b && b < c);
        assert_eq!(c.millis(), 10_000);
        assert_eq!(c.counter(), 2);
    }

    #[test]
    fn counter_carries_into_millis() {
        let mut h = HlcState::new(Hlc(10_000 * 1000 + 999));
        let next = h.tick(10_000);
        assert_eq!(next.millis(), 10_001);
        assert_eq!(next.counter(), 0);
    }

    #[test]
    fn observe_moves_forward_and_rejects_skew() {
        let mut h = HlcState::new(Hlc::ZERO);
        let now = 1_000_000;
        let remote = Hlc::from_millis(now + 60_000);
        h.observe(remote, now).unwrap();
        assert!(h.tick(now) > remote);

        let too_far = Hlc::from_millis(now + MAX_SKEW_MS + 1);
        assert_eq!(h.observe(too_far, now), Err(MAX_SKEW_MS + 1));
    }

    #[test]
    fn hlc_fits_in_js_number() {
        let year_2200 = 7_258_118_400_000_i64;
        assert!(Hlc::from_millis(year_2200).0 + 999 < (1_i64 << 53));
    }
}
