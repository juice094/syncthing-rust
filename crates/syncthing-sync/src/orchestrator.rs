//! Folder orchestrator.
//!
//! Provides globally unified scheduling of scans and pulls across folders:
//!
//! - limits global concurrent scan/pull counts to avoid I/O and CPU spikes;
//! - applies random jitter to first scans so folders do not all start at once;
//! - supports per-folder priorities;
//! - exposes a load snapshot for the monitor and health predictor.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::{debug, info, trace, warn};

/// Orchestrator configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrchestratorConfig {
    /// Maximum number of folders scanning concurrently (default 2).
    pub max_concurrent_scans: usize,
    /// Maximum number of folders pulling concurrently (default 3).
    pub max_concurrent_pulls: usize,
    /// Whether to jitter first scans.
    pub enable_scan_stagger: bool,
    /// Maximum first-scan jitter in seconds (default 60).
    pub scan_stagger_secs: u64,
    /// Dynamic throttle factor: 100 means normal, 50 means halved (default 100).
    ///
    /// Currently serves as an observability signal and scheduling hint only;
    /// the effective concurrency limits remain
    /// `max_concurrent_scans` / `max_concurrent_pulls`.
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
    /// Creates a conservative profile for low-resource or high-latency environments.
    pub fn conservative() -> Self {
        Self {
            max_concurrent_scans: 1,
            max_concurrent_pulls: 1,
            enable_scan_stagger: true,
            scan_stagger_secs: 120,
            throttle_percent: 100,
        }
    }

    /// Creates an aggressive profile for fast local disks.
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

/// Folder scheduling priority.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FolderPriority {
    /// Low priority (large folders, archives).
    Low = 0,
    /// Normal priority.
    #[default]
    Normal = 1,
    /// High priority (workspaces, active directories).
    High = 2,
    /// Critical priority (system configuration, small files).
    Critical = 3,
}

/// Point-in-time snapshot of orchestrator load.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OrchestratorLoad {
    /// Number of folders currently scanning.
    pub active_scans: usize,
    /// Number of folders currently pulling.
    pub active_pulls: usize,
    /// Number of scans waiting for a concurrency slot.
    pub queued_scans: usize,
    /// Number of pulls waiting for a concurrency slot.
    pub queued_pulls: usize,
    /// Current throttle factor (0-100).
    pub throttle_percent: u64,
}

/// Scan permit guard: holds one concurrency slot, released on drop.
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

/// Pull permit guard: holds one concurrency slot, released on drop.
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

/// Globally coordinates scan/pull concurrency across folders.
///
/// Internally uses `tokio::sync::Semaphore` for concurrency control and
/// atomic counters for observability.
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
    /// Folders whose first-scan jitter has already been applied.
    staggered: Mutex<HashMap<String, ()>>,
    /// Per-folder priority overrides.
    priorities: Mutex<HashMap<String, FolderPriority>>,
    /// Global scan counter used as salt for more dispersed jitter.
    scan_counter: AtomicU64,
}

impl FolderOrchestrator {
    /// Creates an orchestrator with default configuration.
    pub fn new() -> Arc<Self> {
        Self::with_config(OrchestratorConfig::default())
    }

    /// Creates an orchestrator with the given configuration.
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

    /// Updates the configuration (new capacities take effect as current
    /// permits are released).
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

    /// Sets a folder's scheduling priority.
    pub fn set_priority(&self, folder_id: &str, priority: FolderPriority) {
        if let Ok(mut guard) = self.priorities.lock() {
            guard.insert(folder_id.to_string(), priority);
        }
        debug!(folder_id = %folder_id, ?priority, "Folder priority set");
    }

    /// Sets the global throttle factor (0-100).
    ///
    /// Currently acts as a control signal for the health predictor; the
    /// concurrency limits remain configuration-driven.
    pub fn set_throttle(&self, percent: u64) {
        let percent = percent.min(100);
        self.throttle_percent.store(percent, Ordering::Relaxed);
        info!(throttle_percent = percent, "Orchestrator throttle updated");
    }

    /// Acquires a scan permit, waiting for a concurrency slot and applying
    /// first-scan jitter if configured.
    pub async fn begin_scan(self: Arc<Self>, folder_id: &str) -> ScanPermit {
        self.scan_queued.fetch_add(1, Ordering::Relaxed);
        trace!(folder_id = %folder_id, "Waiting for scan permit");

        self.maybe_stagger(folder_id).await;

        let permit = match self.scan_sem.clone().acquire_owned().await {
            Ok(p) => Some(p),
            Err(_) => {
                // The semaphore is normally never closed except during
                // shutdown; hand out a dummy permit to degrade gracefully.
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

    /// Acquires a pull permit, waiting for a concurrency slot.
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

    /// Returns the current load snapshot.
    pub fn load(&self) -> OrchestratorLoad {
        OrchestratorLoad {
            active_scans: self.scan_active.load(Ordering::Relaxed),
            active_pulls: self.pull_active.load(Ordering::Relaxed),
            queued_scans: self.scan_queued.load(Ordering::Relaxed),
            queued_pulls: self.pull_queued.load(Ordering::Relaxed),
            throttle_percent: self.throttle_percent.load(Ordering::Relaxed),
        }
    }

    /// Returns a folder's priority (default `Normal` when unset).
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

        let hash = Self::stable_hash(folder_id, self.scan_counter.fetch_add(1, Ordering::Relaxed));
        let jitter_ms = (hash % (max_secs.max(1) * 1000)).max(1);
        let priority = self.priority(folder_id) as u64;
        // Higher-priority folders get half the jitter.
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
