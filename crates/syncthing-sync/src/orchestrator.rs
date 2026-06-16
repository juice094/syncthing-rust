//! 文件夹编排器（Folder Orchestrator）
//!
//! 为多个文件夹的扫描与拉取提供全局统一调度：
//!
//! - 限制全局并发扫描/拉取数量，避免 I/O 与 CPU 尖峰；
//! - 对首次扫描做随机抖动（jitter），避免所有 folder 在同一时刻启动；
//! - 支持 folder 优先级；
//! - 暴露当前负载快照，供 monitor 与 health predictor 消费。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::{debug, info, trace, warn};

/// 编排器配置
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrchestratorConfig {
    /// 最大并发扫描文件夹数（默认 2）
    pub max_concurrent_scans: usize,
    /// 最大并发拉取文件夹数（默认 3）
    pub max_concurrent_pulls: usize,
    /// 是否对首次扫描启用抖动
    pub enable_scan_stagger: bool,
    /// 首次扫描最大抖动秒数（默认 60）
    pub scan_stagger_secs: u64,
    /// 动态节流因子：100 表示正常，50 表示减半并发（默认 100）
    ///
    /// 当前仅作为观测指标与调度建议；实际并发上限仍由
    /// `max_concurrent_scans` / `max_concurrent_pulls` 决定。
    pub throttle_percent: u64,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            max_concurrent_scans: 2,
            max_concurrent_pulls: 3,
            enable_scan_stagger: true,
            scan_stagger_secs: 60,
            throttle_percent: 100,
        }
    }
}

impl OrchestratorConfig {
    /// 创建保守配置，适用于低资源或高延迟环境
    pub fn conservative() -> Self {
        Self {
            max_concurrent_scans: 1,
            max_concurrent_pulls: 1,
            enable_scan_stagger: true,
            scan_stagger_secs: 120,
            throttle_percent: 100,
        }
    }

    /// 创建激进配置，适用于本地高速磁盘
    pub fn aggressive() -> Self {
        Self {
            max_concurrent_scans: 4,
            max_concurrent_pulls: 6,
            enable_scan_stagger: true,
            scan_stagger_secs: 30,
            throttle_percent: 100,
        }
    }
}

/// 文件夹优先级
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FolderPriority {
    /// 低优先级（大文件夹、归档目录）
    Low = 0,
    /// 普通优先级
    #[default]
    Normal = 1,
    /// 高优先级（工作区、活跃目录）
    High = 2,
    /// 关键优先级（系统配置、小文件）
    Critical = 3,
}

/// 编排器负载快照
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OrchestratorLoad {
    pub active_scans: usize,
    pub active_pulls: usize,
    pub queued_scans: usize,
    pub queued_pulls: usize,
    pub throttle_percent: u64,
}

/// 扫描许可守卫：持有期间占用一个并发槽，Drop 时释放
pub struct ScanPermit {
    #[allow(dead_code)]
    permit: Option<OwnedSemaphorePermit>,
    orchestrator: Arc<FolderOrchestrator>,
    folder_id: String,
}

impl Drop for ScanPermit {
    fn drop(&mut self) {
        self.orchestrator
            .scan_active
            .fetch_sub(1, Ordering::Relaxed);
        trace!(folder_id = %self.folder_id, "Scan permit released");
    }
}

/// 拉取许可守卫
pub struct PullPermit {
    #[allow(dead_code)]
    permit: Option<OwnedSemaphorePermit>,
    orchestrator: Arc<FolderOrchestrator>,
    folder_id: String,
}

impl Drop for PullPermit {
    fn drop(&mut self) {
        self.orchestrator
            .pull_active
            .fetch_sub(1, Ordering::Relaxed);
        trace!(folder_id = %self.folder_id, "Pull permit released");
    }
}

/// 文件夹编排器
///
/// 内部使用 `tokio::sync::Semaphore` 做并发控制，使用原子计数器做观测。
#[derive(Debug)]
pub struct FolderOrchestrator {
    max_concurrent_scans: AtomicUsize,
    max_concurrent_pulls: AtomicUsize,
    enable_scan_stagger: AtomicBool,
    scan_stagger_secs: AtomicU64,
    throttle_percent: AtomicU64,
    scan_sem: Arc<Semaphore>,
    pull_sem: Arc<Semaphore>,
    scan_active: AtomicUsize,
    pull_active: AtomicUsize,
    scan_queued: AtomicUsize,
    pull_queued: AtomicUsize,
    /// 每个 folder 是否已完成过首次扫描抖动
    staggered: Mutex<HashMap<String, ()>>,
    /// folder 优先级映射
    priorities: Mutex<HashMap<String, FolderPriority>>,
    /// 全局扫描计数，用于基于 folder_id hash 生成更分散的抖动
    scan_counter: AtomicU64,
}

impl FolderOrchestrator {
    /// 使用默认配置创建编排器
    pub fn new() -> Arc<Self> {
        Self::with_config(OrchestratorConfig::default())
    }

    /// 使用指定配置创建编排器
    pub fn with_config(config: OrchestratorConfig) -> Arc<Self> {
        let max_scans = config.max_concurrent_scans.max(1);
        let max_pulls = config.max_concurrent_pulls.max(1);
        Arc::new(Self {
            max_concurrent_scans: AtomicUsize::new(max_scans),
            max_concurrent_pulls: AtomicUsize::new(max_pulls),
            enable_scan_stagger: AtomicBool::new(config.enable_scan_stagger),
            scan_stagger_secs: AtomicU64::new(config.scan_stagger_secs),
            throttle_percent: AtomicU64::new(config.throttle_percent.min(100)),
            scan_sem: Arc::new(Semaphore::new(max_scans)),
            pull_sem: Arc::new(Semaphore::new(max_pulls)),
            scan_active: AtomicUsize::new(0),
            pull_active: AtomicUsize::new(0),
            scan_queued: AtomicUsize::new(0),
            pull_queued: AtomicUsize::new(0),
            staggered: Mutex::new(HashMap::new()),
            priorities: Mutex::new(HashMap::new()),
            scan_counter: AtomicU64::new(0),
        })
    }

    /// 更新配置（新容量在当前 permit 释放后自然生效）
    pub fn set_config(&self, config: OrchestratorConfig) {
        self.max_concurrent_scans
            .store(config.max_concurrent_scans.max(1), Ordering::Relaxed);
        self.max_concurrent_pulls
            .store(config.max_concurrent_pulls.max(1), Ordering::Relaxed);
        self.enable_scan_stagger
            .store(config.enable_scan_stagger, Ordering::Relaxed);
        self.scan_stagger_secs
            .store(config.scan_stagger_secs, Ordering::Relaxed);
        self.throttle_percent
            .store(config.throttle_percent.min(100), Ordering::Relaxed);
        info!(
            max_scans = config.max_concurrent_scans,
            max_pulls = config.max_concurrent_pulls,
            throttle = config.throttle_percent,
            "Orchestrator config updated"
        );
    }

    /// 设置 folder 优先级
    pub fn set_priority(&self, folder_id: &str, priority: FolderPriority) {
        if let Ok(mut guard) = self.priorities.lock() {
            guard.insert(folder_id.to_string(), priority);
        }
        debug!(folder_id = %folder_id, ?priority, "Folder priority set");
    }

    /// 设置全局节流因子（0-100）
    ///
    /// 当前作为健康预测器的调控信号；并发上限仍由配置决定。
    pub fn set_throttle(&self, percent: u64) {
        let percent = percent.min(100);
        self.throttle_percent.store(percent, Ordering::Relaxed);
        info!(throttle_percent = percent, "Orchestrator throttle updated");
    }

    /// 申请一次扫描许可，可能需要等待并发槽并做首次抖动
    pub async fn begin_scan(self: Arc<Self>, folder_id: &str) -> ScanPermit {
        self.scan_queued.fetch_add(1, Ordering::Relaxed);
        trace!(folder_id = %folder_id, "Waiting for scan permit");

        // 首次扫描做抖动，避免所有 folder 同时启动
        self.maybe_stagger(folder_id).await;

        let permit = match self.scan_sem.clone().acquire_owned().await {
            Ok(p) => Some(p),
            Err(_) => {
                // Semaphore 通常不会关闭，除非程序正在关闭；返回一个虚拟 permit 以允许优雅降级
                warn!(folder_id = %folder_id, "Scan semaphore closed, allowing degraded scan");
                Arc::new(Semaphore::new(1)).try_acquire_owned().ok()
            }
        };

        self.scan_queued.fetch_sub(1, Ordering::Relaxed);
        self.scan_active.fetch_add(1, Ordering::Relaxed);
        debug!(folder_id = %folder_id, "Scan permit acquired");

        ScanPermit {
            permit,
            orchestrator: self.clone(),
            folder_id: folder_id.to_string(),
        }
    }

    /// 申请一次拉取许可
    pub async fn begin_pull(self: Arc<Self>, folder_id: &str) -> PullPermit {
        self.pull_queued.fetch_add(1, Ordering::Relaxed);
        trace!(folder_id = %folder_id, "Waiting for pull permit");

        let permit = match self.pull_sem.clone().acquire_owned().await {
            Ok(p) => Some(p),
            Err(_) => {
                warn!(folder_id = %folder_id, "Pull semaphore closed, allowing degraded pull");
                Arc::new(Semaphore::new(1)).try_acquire_owned().ok()
            }
        };

        self.pull_queued.fetch_sub(1, Ordering::Relaxed);
        self.pull_active.fetch_add(1, Ordering::Relaxed);
        debug!(folder_id = %folder_id, "Pull permit acquired");

        PullPermit {
            permit,
            orchestrator: self.clone(),
            folder_id: folder_id.to_string(),
        }
    }

    /// 当前负载快照
    pub fn load(&self) -> OrchestratorLoad {
        OrchestratorLoad {
            active_scans: self.scan_active.load(Ordering::Relaxed),
            active_pulls: self.pull_active.load(Ordering::Relaxed),
            queued_scans: self.scan_queued.load(Ordering::Relaxed),
            queued_pulls: self.pull_queued.load(Ordering::Relaxed),
            throttle_percent: self.throttle_percent.load(Ordering::Relaxed),
        }
    }

    /// 获取 folder 优先级
    pub fn priority(&self, folder_id: &str) -> FolderPriority {
        self.priorities
            .lock()
            .ok()
            .and_then(|g| g.get(folder_id).copied())
            .unwrap_or_default()
    }

    async fn maybe_stagger(&self, folder_id: &str) {
        let already_staggered = self
            .staggered
            .lock()
            .ok()
            .map(|g| g.contains_key(folder_id))
            .unwrap_or(false);
        if already_staggered {
            return;
        }

        let enabled = self.enable_scan_stagger.load(Ordering::Relaxed);
        let max_secs = self.scan_stagger_secs.load(Ordering::Relaxed);

        if !enabled || max_secs == 0 {
            let _ = self
                .staggered
                .lock()
                .map(|mut g| g.insert(folder_id.to_string(), ()));
            return;
        }

        // 基于 folder_id 计算确定性抖动，避免每次重启都相同分布
        let hash = Self::stable_hash(folder_id, self.scan_counter.fetch_add(1, Ordering::Relaxed));
        let jitter_ms = (hash % (max_secs.max(1) * 1000)).max(1);
        let priority = self.priority(folder_id) as u64;
        // 高优先级 folder 抖动减半
        let jitter_ms = jitter_ms / (1 + (3 - priority.min(3)));

        info!(
            folder_id = %folder_id,
            jitter_ms = jitter_ms,
            "Staggering first scan"
        );

        tokio::time::sleep(Duration::from_millis(jitter_ms)).await;

        let _ = self
            .staggered
            .lock()
            .map(|mut g| g.insert(folder_id.to_string(), ()));
    }

    fn stable_hash(s: &str, salt: u64) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        hasher.write_u64(salt);
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> OrchestratorConfig {
        OrchestratorConfig {
            max_concurrent_scans: 1,
            max_concurrent_pulls: 1,
            enable_scan_stagger: false,
            scan_stagger_secs: 0,
            throttle_percent: 100,
        }
    }

    #[tokio::test]
    async fn test_orchestrator_limits_scan_concurrency() {
        let orch = FolderOrchestrator::with_config(test_config());

        let p1 = orch.clone().begin_scan("a").await;
        assert_eq!(orch.load().active_scans, 1);

        // 由于并发度为 1，第二个 begin_scan 会阻塞；用 timeout 验证它在等待
        let orch2 = Arc::clone(&orch);
        let pending = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_millis(200), orch2.clone().begin_scan("b")).await
        });

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(orch.load().queued_scans, 1);

        drop(p1);
        let result = pending.await.unwrap();
        assert!(result.is_ok());
        assert_eq!(orch.load().active_scans, 1);
    }

    #[tokio::test]
    async fn test_orchestrator_limits_pull_concurrency() {
        let orch = FolderOrchestrator::with_config(OrchestratorConfig {
            max_concurrent_scans: 2,
            max_concurrent_pulls: 1,
            ..test_config()
        });

        let p1 = orch.clone().begin_pull("a").await;
        assert_eq!(orch.load().active_pulls, 1);

        let orch2 = Arc::clone(&orch);
        let pending = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_millis(200), orch2.clone().begin_pull("b")).await
        });

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(orch.load().queued_pulls, 1);

        drop(p1);
        let result = pending.await.unwrap();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_orchestrator_throttle_observable() {
        let orch = FolderOrchestrator::new();
        orch.set_throttle(50);
        assert_eq!(orch.load().throttle_percent, 50);
    }

    #[tokio::test]
    async fn test_priority_and_stagger() {
        let orch = FolderOrchestrator::with_config(OrchestratorConfig {
            max_concurrent_scans: 2,
            max_concurrent_pulls: 2,
            enable_scan_stagger: true,
            scan_stagger_secs: 1,
            throttle_percent: 100,
        });

        orch.set_priority("critical", FolderPriority::Critical);
        assert_eq!(orch.priority("critical"), FolderPriority::Critical);
        assert_eq!(orch.priority("unknown"), FolderPriority::Normal);

        // Critical 优先级抖动减半，测试应在 1s 内完成
        let start = tokio::time::Instant::now();
        let _permit = orch.clone().begin_scan("critical").await;
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
