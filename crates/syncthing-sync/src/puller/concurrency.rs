//! Concurrency policy.
//!
//! Dynamically adjusts Puller concurrency based on link quality (RTT,
//! direct/relay).

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Shared reference to a concurrency policy.
pub type ConcurrencyPolicyRef = Arc<ConcurrencyPolicy>;

/// Default low-concurrency profile (conservative, friendly to high-latency / relay links).
const DEFAULT_DOWNLOADS: usize = 2;
const DEFAULT_BLOCKS: usize = 4;

/// Low-quality link profile: >300 ms or unknown RTT.
const LOW_DOWNLOADS: usize = 2;
const LOW_BLOCKS: usize = 4;

/// Medium-quality link profile: 50-300 ms.
const MEDIUM_DOWNLOADS: usize = 4;
const MEDIUM_BLOCKS: usize = 8;

/// High-quality link profile: 1-50 ms.
const HIGH_DOWNLOADS: usize = 8;
const HIGH_BLOCKS: usize = 16;

/// Direct-link profile.
const DIRECT_DOWNLOADS: usize = 6;
const DIRECT_BLOCKS: usize = 12;

/// Puller concurrency policy.
///
/// Internally uses `AtomicUsize` so reads are lock-free, suitable for the
/// high-frequency pull path.
#[derive(Debug, Default)]
pub struct ConcurrencyPolicy {
    downloads: AtomicUsize,
    blocks: AtomicUsize,
}

impl ConcurrencyPolicy {
    /// Creates a policy with the default low concurrency (2 files / 4 blocks).
    pub fn new() -> Self {
        Self::with_defaults(DEFAULT_DOWNLOADS, DEFAULT_BLOCKS)
    }

    /// Creates a policy with the given default limits.
    pub fn with_defaults(downloads: usize, blocks: usize) -> Self {
        Self {
            downloads: AtomicUsize::new(downloads.max(1)),
            blocks: AtomicUsize::new(blocks.max(1)),
        }
    }

    /// Returns the current max concurrent file downloads.
    pub fn downloads(&self) -> usize {
        self.downloads.load(Ordering::Relaxed)
    }

    /// Returns the current max concurrent block requests per file.
    pub fn blocks(&self) -> usize {
        self.blocks.load(Ordering::Relaxed)
    }

    /// Sets both file-level and block-level concurrency limits.
    fn set(&self, downloads: usize, blocks: usize) {
        self.downloads.store(downloads.max(1), Ordering::Relaxed);
        self.blocks.store(blocks.max(1), Ordering::Relaxed);
    }

    /// Sets the concurrency tier from the RTT in milliseconds.
    ///
    /// - 0 ms           → 2 files / 4 blocks (conservative default)
    /// - 1-50 ms        → 8 files / 16 blocks
    /// - 51-300 ms      → 4 files / 8 blocks
    /// - >300 ms or unknown → 2 files / 4 blocks
    pub fn set_for_rtt(&self, rtt_ms: u64) {
        match rtt_ms {
            0 => self.set(LOW_DOWNLOADS, LOW_BLOCKS),
            1..=50 => self.set(HIGH_DOWNLOADS, HIGH_BLOCKS),
            51..=300 => self.set(MEDIUM_DOWNLOADS, MEDIUM_BLOCKS),
            _ => self.set(LOW_DOWNLOADS, LOW_BLOCKS),
        }
    }

    /// Sets the high-concurrency direct-link tier (6 files / 12 blocks).
    pub fn set_for_direct(&self) {
        self.set(DIRECT_DOWNLOADS, DIRECT_BLOCKS);
    }

    /// Sets the conservative relay-link tier (2 files / 4 blocks).
    pub fn set_for_relay(&self) {
        self.set(LOW_DOWNLOADS, LOW_BLOCKS);
    }
}

/// Shared RTT tracker.
///
/// Maintains an exponentially moving average RTT (milliseconds) that
/// `ConcurrencyPolicy` reads periodically.
#[derive(Debug, Default)]
pub struct RttTracker {
    rtt_ms: AtomicU64,
}

impl RttTracker {
    /// Creates a new tracker; the RTT starts out unknown.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            // An unknown RTT is represented as u64::MAX; the policy then
            // falls back to the lowest tier.
            rtt_ms: AtomicU64::new(u64::MAX),
        })
    }

    /// Records an RTT sample and updates the exponential moving average.
    ///
    /// Uses `avg = (avg * 7 + sample) / 8` to smooth out bursts.
    pub fn sample(&self, rtt: Duration) {
        let sample_ms = rtt.as_millis() as u64;
        let old = self.rtt_ms.load(Ordering::Relaxed);
        let new = if old == u64::MAX {
            sample_ms
        } else {
            (old * 7 + sample_ms) / 8
        };
        self.rtt_ms.store(new, Ordering::Relaxed);
    }

    /// Returns the current average RTT in milliseconds (`u64::MAX` when unknown).
    pub fn rtt_ms(&self) -> u64 {
        self.rtt_ms.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_values() {
        let policy = ConcurrencyPolicy::new();
        assert_eq!(policy.downloads(), DEFAULT_DOWNLOADS);
        assert_eq!(policy.blocks(), DEFAULT_BLOCKS);
    }

    #[test]
    fn test_with_defaults() {
        let policy = ConcurrencyPolicy::with_defaults(10, 20);
        assert_eq!(policy.downloads(), 10);
        assert_eq!(policy.blocks(), 20);
    }

    #[test]
    fn test_set_for_rtt_tiers() {
        let policy = ConcurrencyPolicy::new();

        policy.set_for_rtt(0);
        assert_eq!(policy.downloads(), LOW_DOWNLOADS);
        assert_eq!(policy.blocks(), LOW_BLOCKS);

        policy.set_for_rtt(30);
        assert_eq!(policy.downloads(), HIGH_DOWNLOADS);
        assert_eq!(policy.blocks(), HIGH_BLOCKS);

        policy.set_for_rtt(100);
        assert_eq!(policy.downloads(), MEDIUM_DOWNLOADS);
        assert_eq!(policy.blocks(), MEDIUM_BLOCKS);

        policy.set_for_rtt(500);
        assert_eq!(policy.downloads(), LOW_DOWNLOADS);
        assert_eq!(policy.blocks(), LOW_BLOCKS);

        policy.set_for_rtt(u64::MAX);
        assert_eq!(policy.downloads(), LOW_DOWNLOADS);
        assert_eq!(policy.blocks(), LOW_BLOCKS);
    }

    #[test]
    fn test_direct_and_relay_helpers() {
        let policy = ConcurrencyPolicy::new();
        policy.set_for_direct();
        assert_eq!(policy.downloads(), DIRECT_DOWNLOADS);
        assert_eq!(policy.blocks(), DIRECT_BLOCKS);

        policy.set_for_relay();
        assert_eq!(policy.downloads(), LOW_DOWNLOADS);
        assert_eq!(policy.blocks(), LOW_BLOCKS);
    }

    #[test]
    fn test_rtt_tracker() {
        let tracker = RttTracker::new();
        assert_eq!(tracker.rtt_ms(), u64::MAX);

        tracker.sample(Duration::from_millis(100));
        assert_eq!(tracker.rtt_ms(), 100);

        tracker.sample(Duration::from_millis(200));
        assert_eq!(tracker.rtt_ms(), (100 * 7 + 200) / 8);
    }
}
