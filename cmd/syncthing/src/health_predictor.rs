//! 预测性健康检查（Predictive Health Checks）
//!
//! 订阅同步事件并周期性评估趋势，提前发现潜在问题：
//!
//! - 扫描/拉取失败率上升
//! - Watcher 事件丢失
//! - 文件夹状态频繁翻转（flapping）
//! - 增量扫描占比下降（脏路径集合过大）
//!
//! 当检测到异常趋势时，向日志输出警告，并可对 FolderOrchestrator 进行节流。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use syncthing_sync::events::ItemAction;
use syncthing_sync::{EventSubscriber, FolderOrchestrator, SyncEvent};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{info, warn};

/// 健康检查配置
#[derive(Debug, Clone, Copy)]
pub struct HealthPredictorConfig {
    /// 评估间隔（默认 60 秒）
    pub interval_secs: u64,
    /// 滑动窗口包含的评估间隔数（默认 5）
    pub window_intervals: usize,
    /// 扫描失败率阈值（默认 10%）
    pub scan_failure_ratio_threshold: f64,
    /// 拉取失败率阈值（默认 10%）
    pub pull_failure_ratio_threshold: f64,
    /// 每个窗口内 watcher 丢事件阈值（默认 50）
    pub watcher_drop_threshold: u64,
    /// 每个窗口内状态翻转阈值（默认 20）
    pub state_flap_threshold: u64,
    /// 增量扫描占比阈值（默认 0.5）；低于该值说明 watcher 脏集合过大
    pub incremental_ratio_threshold: f64,
}

impl Default for HealthPredictorConfig {
    fn default() -> Self {
        Self {
            interval_secs: 60,
            window_intervals: 5,
            scan_failure_ratio_threshold: 0.10,
            pull_failure_ratio_threshold: 0.10,
            watcher_drop_threshold: 50,
            state_flap_threshold: 20,
            incremental_ratio_threshold: 0.5,
        }
    }
}

/// 单个 folder 在一个窗口内的观测计数
#[derive(Debug, Default, Clone, Copy)]
struct FolderWindow {
    scans_completed: u64,
    scans_failed: u64,
    pulls_completed: u64,
    pulls_failed: u64,
    watcher_dropped: u64,
    incremental_scans: u64,
    full_scans: u64,
    state_transitions: u64,
}

/// 预测性健康检查器
#[derive(Debug)]
pub struct HealthPredictor {
    subscriber: EventSubscriber,
    orchestrator: Option<Arc<FolderOrchestrator>>,
    config: HealthPredictorConfig,
    global_scans_completed: AtomicU64,
    global_pulls_completed: AtomicU64,
}

impl HealthPredictor {
    /// 创建健康检查器
    pub fn new(subscriber: EventSubscriber, orchestrator: Option<Arc<FolderOrchestrator>>) -> Self {
        Self::with_config(subscriber, orchestrator, HealthPredictorConfig::default())
    }

    /// 使用指定配置创建健康检查器
    pub fn with_config(
        subscriber: EventSubscriber,
        orchestrator: Option<Arc<FolderOrchestrator>>,
        config: HealthPredictorConfig,
    ) -> Self {
        Self {
            subscriber,
            orchestrator,
            config,
            global_scans_completed: AtomicU64::new(0),
            global_pulls_completed: AtomicU64::new(0),
        }
    }

    /// 在后台启动健康检查任务
    pub fn spawn(mut self, mut shutdown: watch::Receiver<bool>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_secs(self.config.interval_secs));
            let mut current: HashMap<String, FolderWindow> = HashMap::new();
            let mut history: Vec<HashMap<String, FolderWindow>> = Vec::new();

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // 汇总当前窗口与历史窗口
                        let aggregated = Self::aggregate_windows(&current, &history, self.config.window_intervals);
                        self.evaluate(&aggregated);

                        history.push(current);
                        if history.len() > self.config.window_intervals {
                            history.remove(0);
                        }
                        current = HashMap::new();
                    }
                    Some(event) = self.subscriber.recv() => {
                        Self::record_event(&mut current, &event, &self.global_scans_completed, &self.global_pulls_completed);
                    }
                    _ = shutdown.changed() => {
                        if *shutdown.borrow() {
                            info!("Health predictor shutting down");
                            break;
                        }
                    }
                }
            }
        })
    }

    fn record_event(
        current: &mut HashMap<String, FolderWindow>,
        event: &SyncEvent,
        global_scans: &AtomicU64,
        global_pulls: &AtomicU64,
    ) {
        match event {
            SyncEvent::FolderScanCompleted { folder, .. } => {
                current.entry(folder.clone()).or_default().scans_completed += 1;
                global_scans.fetch_add(1, Ordering::Relaxed);
            }
            SyncEvent::FolderScanFailed { folder, .. } => {
                current.entry(folder.clone()).or_default().scans_failed += 1;
            }
            SyncEvent::SyncComplete { folder, .. } => {
                current.entry(folder.clone()).or_default().pulls_completed += 1;
                global_pulls.fetch_add(1, Ordering::Relaxed);
            }
            SyncEvent::ItemFinished {
                folder,
                action: ItemAction::Modify | ItemAction::Add,
                error: Some(_),
                ..
            } => {
                current.entry(folder.clone()).or_default().pulls_failed += 1;
            }
            SyncEvent::WatcherEventsDropped { folder, dropped } => {
                current.entry(folder.clone()).or_default().watcher_dropped += *dropped;
            }
            SyncEvent::IncrementalScanTriggered { folder, .. } => {
                current.entry(folder.clone()).or_default().incremental_scans += 1;
            }
            SyncEvent::FolderStateChanged { folder, from, to } if from != to => {
                current.entry(folder.clone()).or_default().state_transitions += 1;
            }
            _ => {}
        }
    }

    fn aggregate_windows(
        current: &HashMap<String, FolderWindow>,
        history: &[HashMap<String, FolderWindow>],
        max_intervals: usize,
    ) -> HashMap<String, FolderWindow> {
        let mut result: HashMap<String, FolderWindow> = current.clone();
        let start = history
            .len()
            .saturating_sub(max_intervals.saturating_sub(1));
        for window in &history[start..] {
            for (folder, w) in window {
                let entry = result.entry(folder.clone()).or_default();
                entry.scans_completed += w.scans_completed;
                entry.scans_failed += w.scans_failed;
                entry.pulls_completed += w.pulls_completed;
                entry.pulls_failed += w.pulls_failed;
                entry.watcher_dropped += w.watcher_dropped;
                entry.incremental_scans += w.incremental_scans;
                entry.full_scans += w.full_scans;
                entry.state_transitions += w.state_transitions;
            }
        }
        result
    }

    fn evaluate(&self, aggregated: &HashMap<String, FolderWindow>) {
        let mut any_alert = false;
        for (folder, w) in aggregated {
            let scan_total = w.scans_completed + w.scans_failed;
            let scan_fail_ratio = if scan_total > 0 {
                w.scans_failed as f64 / scan_total as f64
            } else {
                0.0
            };

            let pull_total = w.pulls_completed + w.pulls_failed;
            let pull_fail_ratio = if pull_total > 0 {
                w.pulls_failed as f64 / pull_total as f64
            } else {
                0.0
            };

            let incremental_total = w.incremental_scans + w.full_scans;
            let incremental_ratio = if incremental_total > 0 {
                w.incremental_scans as f64 / incremental_total as f64
            } else {
                1.0
            };

            if scan_fail_ratio > self.config.scan_failure_ratio_threshold {
                warn!(
                    folder = %folder,
                    ratio = %format!("{:.2}", scan_fail_ratio),
                    "Predictive alert: scan failure rate is elevated"
                );
                any_alert = true;
            }

            if pull_fail_ratio > self.config.pull_failure_ratio_threshold {
                warn!(
                    folder = %folder,
                    ratio = %format!("{:.2}", pull_fail_ratio),
                    "Predictive alert: pull failure rate is elevated"
                );
                any_alert = true;
            }

            if w.watcher_dropped > self.config.watcher_drop_threshold {
                warn!(
                    folder = %folder,
                    dropped = w.watcher_dropped,
                    "Predictive alert: watcher events dropped, filesystem pressure high"
                );
                any_alert = true;
            }

            if w.state_transitions > self.config.state_flap_threshold {
                warn!(
                    folder = %folder,
                    transitions = w.state_transitions,
                    "Predictive alert: folder state flapping"
                );
                any_alert = true;
            }

            if incremental_ratio < self.config.incremental_ratio_threshold && incremental_total > 0
            {
                warn!(
                    folder = %folder,
                    ratio = %format!("{:.2}", incremental_ratio),
                    "Predictive alert: incremental scan ratio low, dirty set may be too large"
                );
                any_alert = true;
            }
        }

        // 当检测到异常时，对 orchestrator 进行节流；无异常时逐步恢复
        if let Some(ref orch) = self.orchestrator {
            let current_throttle = orch.load().throttle_percent;
            if any_alert && current_throttle > 50 {
                let new_throttle = current_throttle.saturating_sub(20).max(50);
                orch.set_throttle(new_throttle);
                warn!(
                    throttle_percent = new_throttle,
                    "Health predictor throttling orchestrator due to alerts"
                );
            } else if !any_alert && current_throttle < 100 {
                let new_throttle = (current_throttle + 10).min(100);
                orch.set_throttle(new_throttle);
                info!(
                    throttle_percent = new_throttle,
                    "Health predictor recovering orchestrator throttle"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syncthing_sync::EventPublisher;

    #[tokio::test]
    async fn test_record_scan_failure() {
        let publisher = EventPublisher::new(10);
        let subscriber = publisher.subscribe();
        let predictor = HealthPredictor::new(subscriber, None);

        // 使用一个独立订阅者收集测试事件
        let mut test_sub = publisher.subscribe();

        publisher.publish(SyncEvent::FolderScanCompleted {
            folder: "f".to_string(),
            files_changed: 1,
        });
        publisher.publish(SyncEvent::FolderScanFailed {
            folder: "f".to_string(),
            error: "io".to_string(),
        });

        let mut current = HashMap::new();
        while let Ok(event) = test_sub.try_recv() {
            HealthPredictor::record_event(
                &mut current,
                &event,
                &predictor.global_scans_completed,
                &predictor.global_pulls_completed,
            );
        }

        let w = current.get("f").copied().unwrap_or_default();
        assert_eq!(w.scans_completed, 1);
        assert_eq!(w.scans_failed, 1);
    }
}
