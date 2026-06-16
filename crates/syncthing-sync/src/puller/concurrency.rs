//! 并发策略
//!
//! 根据连接质量（RTT、直连/中继）动态调整 Puller 并发度。

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// 共享并发策略引用
pub type ConcurrencyPolicyRef = Arc<ConcurrencyPolicy>;

/// 默认低并发配置：保守默认值，高延迟/中继链路友好
const DEFAULT_DOWNLOADS: usize = 2;
const DEFAULT_BLOCKS: usize = 4;

/// 低质量连接配置：>300ms 或未知 RTT
const LOW_DOWNLOADS: usize = 2;
const LOW_BLOCKS: usize = 4;

/// 中等质量连接配置：50-300ms
const MEDIUM_DOWNLOADS: usize = 4;
const MEDIUM_BLOCKS: usize = 8;

/// 高质量连接配置：1-50ms
const HIGH_DOWNLOADS: usize = 8;
const HIGH_BLOCKS: usize = 16;

/// 直连链路配置
const DIRECT_DOWNLOADS: usize = 6;
const DIRECT_BLOCKS: usize = 12;

/// Puller 并发策略
///
/// 内部使用 `AtomicUsize`，读取为 lock-free，适合高频 pull 路径。
#[derive(Debug, Default)]
pub struct ConcurrencyPolicy {
    downloads: AtomicUsize,
    blocks: AtomicUsize,
}

impl ConcurrencyPolicy {
    /// 创建策略，初始为默认低并发（2 文件 / 4 块）
    pub fn new() -> Self {
        Self::with_defaults(DEFAULT_DOWNLOADS, DEFAULT_BLOCKS)
    }

    /// 使用指定默认值创建策略
    pub fn with_defaults(downloads: usize, blocks: usize) -> Self {
        Self {
            downloads: AtomicUsize::new(downloads.max(1)),
            blocks: AtomicUsize::new(blocks.max(1)),
        }
    }

    /// 当前最大并发下载文件数
    pub fn downloads(&self) -> usize {
        self.downloads.load(Ordering::Relaxed)
    }

    /// 当前单文件最大并发块请求数
    pub fn blocks(&self) -> usize {
        self.blocks.load(Ordering::Relaxed)
    }

    /// 同时设置文件级与块级并发上限
    fn set(&self, downloads: usize, blocks: usize) {
        self.downloads.store(downloads.max(1), Ordering::Relaxed);
        self.blocks.store(blocks.max(1), Ordering::Relaxed);
    }

    /// 根据 RTT（毫秒）设置并发档位
    ///
    /// - 0 ms        → 2 文件 / 4 块（保守默认值）
    /// - 1-50 ms     → 8 文件 / 16 块
    /// - 51-300 ms   → 4 文件 / 8 块
    /// - >300 ms 或未知 → 2 文件 / 4 块
    pub fn set_for_rtt(&self, rtt_ms: u64) {
        match rtt_ms {
            0 => self.set(LOW_DOWNLOADS, LOW_BLOCKS),
            1..=50 => self.set(HIGH_DOWNLOADS, HIGH_BLOCKS),
            51..=300 => self.set(MEDIUM_DOWNLOADS, MEDIUM_BLOCKS),
            _ => self.set(LOW_DOWNLOADS, LOW_BLOCKS),
        }
    }

    /// 设置为直连链路的高并发档位（6 文件 / 12 块）
    pub fn set_for_direct(&self) {
        self.set(DIRECT_DOWNLOADS, DIRECT_BLOCKS);
    }

    /// 设置为中继链路的保守档位（2 文件 / 4 块）
    pub fn set_for_relay(&self) {
        self.set(LOW_DOWNLOADS, LOW_BLOCKS);
    }
}

/// 共享 RTT 跟踪器
///
/// 维护一个指数移动平均 RTT（毫秒），供 `ConcurrencyPolicy` 周期性读取。
#[derive(Debug, Default)]
pub struct RttTracker {
    rtt_ms: AtomicU64,
}

impl RttTracker {
    /// 创建新的 RTT 跟踪器，初始未知
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            // 未知 RTT 用 u64::MAX 表示，策略会回退到最低档位
            rtt_ms: AtomicU64::new(u64::MAX),
        })
    }

    /// 记录一次 RTT 样本，并更新指数移动平均
    ///
    /// 使用 `avg = (avg * 7 + sample) / 8` 平滑突发波动。
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

    /// 获取当前平均 RTT（毫秒），未知时返回 `u64::MAX`
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
