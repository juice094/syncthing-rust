//! Byte-rate throttling via a hand-rolled token bucket.
//!
//! See `docs/design/ADR-20261106-bandwidth-throttle.md`. We intentionally do
//! not pull in `governor` or any other rate-limiting crate: the project's hard
//! rule is "new dependencies only when unavoidable", and a single-bucket
//! limiter is small enough to own.
//!
//! A rate of `0` or `None` means "unlimited" and every call is a no-op.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A token bucket that hands out byte permits at a fixed rate.
///
/// The bucket starts full so a burst can be sent immediately; afterwards
/// `acquire` sleeps until enough tokens have been refilled.
#[derive(Debug)]
pub struct TokenBucket {
    /// Refill rate in bytes per second. `0` disables throttling.
    rate_bytes_per_sec: u64,
    /// Maximum tokens the bucket can hold (burst ceiling).
    capacity: u64,
    state: Mutex<BucketState>,
}

#[derive(Debug)]
struct BucketState {
    tokens: u64,
    last_refill: Instant,
}

impl TokenBucket {
    /// Creates an unlimited bucket (all `acquire` calls return immediately).
    pub fn unlimited() -> Self {
        Self {
            rate_bytes_per_sec: 0,
            capacity: 0,
            state: Mutex::new(BucketState {
                tokens: 0,
                last_refill: Instant::now(),
            }),
        }
    }

    /// Creates a bucket from a KiB/s rate. `None` or `Some(0)` yields unlimited.
    ///
    /// Burst capacity is twice the per-second rate, which avoids starving
    /// small requests while still smoothing large transfers.
    pub fn from_kbps(kbps: Option<u64>) -> Self {
        match kbps {
            None | Some(0) => Self::unlimited(),
            Some(kbps) => {
                let rate_bytes_per_sec = kbps.saturating_mul(1024);
                let capacity = rate_bytes_per_sec.saturating_mul(2);
                Self {
                    rate_bytes_per_sec,
                    capacity,
                    state: Mutex::new(BucketState {
                        tokens: capacity,
                        last_refill: Instant::now(),
                    }),
                }
            }
        }
    }

    /// Returns `true` when throttling is disabled.
    pub fn is_unlimited(&self) -> bool {
        self.rate_bytes_per_sec == 0
    }

    /// Waits until `n` bytes worth of tokens are available, then consumes them.
    ///
    /// Requests larger than `capacity` are clamped to `capacity` so a single
    /// huge block cannot deadlock the bucket; the remainder is effectively
    /// granted as burst.
    pub async fn acquire(&self, n: u64) {
        if self.is_unlimited() || n == 0 {
            return;
        }

        let wait = {
            let mut state = match self.state.lock() {
                Ok(guard) => guard,
                // A poisoned mutex means a previous holder panicked. Degrade to
                // unlimited rather than propagating the panic into the sync path.
                Err(poisoned) => poisoned.into_inner(),
            };

            let now = Instant::now();
            let elapsed = now.duration_since(state.last_refill);
            let refilled =
                (elapsed.as_nanos() as u64).saturating_mul(self.rate_bytes_per_sec) / 1_000_000_000;
            if refilled > 0 {
                state.tokens = state.tokens.saturating_add(refilled).min(self.capacity);
                state.last_refill = now;
            }

            let want = n.min(self.capacity);
            if state.tokens >= want {
                state.tokens -= want;
                None
            } else {
                let deficit = want - state.tokens;
                // Drain the bucket and compute how long until `deficit` refills.
                state.tokens = 0;
                state.last_refill = now;
                let nanos = (deficit as u128) * 1_000_000_000 / (self.rate_bytes_per_sec as u128);
                Some(Duration::from_nanos(nanos.min(u64::MAX as u128) as u64))
            }
        };

        if let Some(duration) = wait {
            tokio::time::sleep(duration).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unlimited_bucket_returns_immediately() {
        let bucket = TokenBucket::unlimited();
        assert!(bucket.is_unlimited());
        let start = Instant::now();
        bucket.acquire(10 * 1024 * 1024).await;
        assert!(start.elapsed() < Duration::from_millis(10));
    }

    #[tokio::test]
    async fn none_rate_is_unlimited() {
        assert!(TokenBucket::from_kbps(None).is_unlimited());
        assert!(TokenBucket::from_kbps(Some(0)).is_unlimited());
    }

    #[tokio::test]
    async fn zero_byte_request_does_not_sleep() {
        let bucket = TokenBucket::from_kbps(Some(1));
        let start = Instant::now();
        bucket.acquire(0).await;
        assert!(start.elapsed() < Duration::from_millis(10));
    }

    #[tokio::test]
    async fn burst_up_to_capacity_is_immediate() {
        // 1 KiB/s => capacity 2048 bytes; the bucket starts full.
        let bucket = TokenBucket::from_kbps(Some(1));
        let start = Instant::now();
        bucket.acquire(2048).await;
        assert!(start.elapsed() < Duration::from_millis(20));
    }

    #[tokio::test]
    async fn exceeding_capacity_sleeps_roughly_proportionally() {
        // 1000 KiB/s => 1_024_000 B/s, capacity 2_048_000.
        // Drain the bucket with one capacity-sized request, then request
        // another 512_000 bytes: should wait ~0.5s.
        let bucket = TokenBucket::from_kbps(Some(1000));
        bucket.acquire(2_048_000).await;

        let start = Instant::now();
        bucket.acquire(512_000).await;
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(450) && elapsed < Duration::from_millis(700),
            "expected ~500ms wait, got {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn request_larger_than_capacity_is_clamped() {
        let bucket = TokenBucket::from_kbps(Some(1));
        // Request 10x capacity; it should drain and wait for one capacity refill
        // (~2s at 1 KiB/s), not explode or hang.
        let start = Instant::now();
        bucket.acquire(20_480).await;
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
