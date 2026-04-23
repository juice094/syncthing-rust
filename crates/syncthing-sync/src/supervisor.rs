//! Supervisor for long-lived async tasks
//!
//! A Rust equivalent of Go `suture.Supervisor` that supervises async tasks with
//! automatic restart, exponential backoff, and max-restart limits.

use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
/// Boxed error type used by supervised tasks.
pub type BoxError = Box<dyn Error + Send + Sync>;

/// Type alias for a factory that produces supervised task futures.
pub type TaskFactory = Box<
    dyn Fn() -> Pin<Box<dyn Future<Output = Result<(), BoxError>> + Send>> + Send + Sync,
>;

/// Type alias for a permanent-failure callback.
pub type FailureCallback = Arc<dyn Fn(&str) + Send + Sync>;

/// Restart policy for a supervised task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    /// Always restart the task when it finishes, regardless of result.
    Always,
    /// Only restart when the task returns an error or panics.
    OnFailure,
    /// Never restart the task.
    Never,
}

/// Exponential backoff configuration.
#[derive(Debug, Clone, Copy)]
pub struct ExponentialBackoff {
    /// Initial delay before the first restart.
    pub initial_delay: Duration,
    /// Maximum delay between restarts.
    pub max_delay: Duration,
    /// Duration after which the restart counter resets.
    pub reset_after: Duration,
}

impl ExponentialBackoff {
    /// Compute the next backoff delay for the given attempt number.
    pub fn next_delay(&self, attempt: u32) -> Duration {
        let multiplier = 2u32.saturating_pow(attempt.min(31));
        self.initial_delay.saturating_mul(multiplier).min(self.max_delay)
    }
}

/// Configuration controlling how a task is restarted.
#[derive(Debug, Clone)]
pub struct RestartConfig {
    /// When to restart the task.
    pub restart_policy: RestartPolicy,
    /// Backoff parameters.
    pub backoff: ExponentialBackoff,
    /// Maximum number of restarts allowed within the `reset_after` window.
    pub max_restarts: u32,
}

impl Default for RestartConfig {
    fn default() -> Self {
        Self {
            restart_policy: RestartPolicy::OnFailure,
            backoff: ExponentialBackoff {
                initial_delay: Duration::from_millis(100),
                max_delay: Duration::from_secs(60),
                reset_after: Duration::from_secs(60),
            },
            max_restarts: 5,
        }
    }
}

/// A task to be supervised.
pub struct SupervisedTask {
    /// Human-readable name used for logging and callbacks.
    pub name: String,
    /// Factory that produces a new future each time the task is (re)started.
    pub future_factory: TaskFactory,
    /// Restart configuration.
    pub config: RestartConfig,
}

/// Supervisor that manages a collection of `SupervisedTask`s.
pub struct Supervisor {
    tasks: Vec<SupervisedTask>,
    on_permanent_failure: Option<FailureCallback>,
    handles: Vec<JoinHandle<()>>,
    shutdown_tx: broadcast::Sender<()>,
}

impl Supervisor {
    /// Create a new, empty supervisor.
    pub fn new() -> Self {
        let (shutdown_tx, _) = broadcast::channel(1);
        Self {
            tasks: Vec::new(),
            on_permanent_failure: None,
            handles: Vec::new(),
            shutdown_tx,
        }
    }

    /// Register a callback invoked when a task exceeds `max_restarts`.
    pub fn on_permanent_failure<F>(&mut self, callback: F)
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        self.on_permanent_failure = Some(Arc::new(callback));
    }

    /// Add a task to be supervised.
    pub fn add_task(&mut self, task: SupervisedTask) {
        self.tasks.push(task);
    }

    /// Start all registered tasks. Each task is spawned on the current Tokio runtime.
    pub fn start(&mut self) {
        while let Some(task) = self.tasks.pop() {
            let shutdown_rx = self.shutdown_tx.subscribe();
            let on_failure = self.on_permanent_failure.clone();
            let handle = tokio::spawn(supervise_task(task, shutdown_rx, on_failure));
            self.handles.push(handle);
        }
    }

    /// Gracefully shut down the supervisor by aborting all supervised tasks.
    pub async fn shutdown(mut self) {
        let _ = self.shutdown_tx.send(());
        for handle in self.handles.drain(..) {
            handle.abort();
            let _ = handle.await;
        }
    }
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::new()
    }
}

async fn supervise_task(
    task: SupervisedTask,
    mut shutdown_rx: broadcast::Receiver<()>,
    on_permanent_failure: Option<FailureCallback>,
) {
    let mut attempt: u32 = 0;
    let mut restart_count: u32 = 0;
    let mut window_start = Instant::now();

    loop {
        if attempt > 0 {
            let delay = task.config.backoff.next_delay(attempt - 1);
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = shutdown_rx.recv() => {
                    return;
                }
            }
        }

        let fut = (task.future_factory)();
        let handle = tokio::spawn(fut);
        let abort_handle = handle.abort_handle();

        tokio::select! {
            res = handle => {
                let should_restart = match &res {
                    Ok(Ok(())) => matches!(task.config.restart_policy, RestartPolicy::Always),
                    Ok(Err(_)) => {
                        matches!(task.config.restart_policy, RestartPolicy::Always | RestartPolicy::OnFailure)
                    }
                    Err(_) => {
                        matches!(task.config.restart_policy, RestartPolicy::Always | RestartPolicy::OnFailure)
                    }
                };

                if !should_restart {
                    return;
                }

                let now = Instant::now();
                if now.duration_since(window_start) > task.config.backoff.reset_after {
                    window_start = now;
                    restart_count = 0;
                }

                restart_count += 1;
                if restart_count > task.config.max_restarts {
                    if let Some(cb) = &on_permanent_failure {
                        cb(&task.name);
                    }
                    return;
                }

                attempt += 1;
            }
            _ = shutdown_rx.recv() => {
                abort_handle.abort();
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_supervisor_restarts_on_panic() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter2 = counter.clone();

        let mut supervisor = Supervisor::new();
        supervisor.add_task(SupervisedTask {
            name: "panic-task".to_string(),
            future_factory: Box::new(move || {
                let c = counter2.clone();
                Box::pin(async move {
                    let n = c.fetch_add(1, Ordering::SeqCst);
                    if n == 0 {
                        panic!("intentional panic");
                    }
                    Ok(())
                })
            }),
            config: RestartConfig {
                restart_policy: RestartPolicy::OnFailure,
                ..Default::default()
            },
        });
        supervisor.start();

        sleep(Duration::from_millis(500)).await;

        let count = counter.load(Ordering::SeqCst);
        assert!(count >= 2, "expected at least 2 invocations, got {}", count);

        supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn test_supervisor_backoff_increases() {
        let times = Arc::new(std::sync::Mutex::new(Vec::new()));
        let times2 = times.clone();

        let mut supervisor = Supervisor::new();
        supervisor.add_task(SupervisedTask {
            name: "backoff-task".to_string(),
            future_factory: Box::new(move || {
                let t = times2.clone();
                Box::pin(async move {
                    t.lock().unwrap().push(Instant::now());
                    Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "fail")) as BoxError)
                })
            }),
            config: RestartConfig {
                restart_policy: RestartPolicy::OnFailure,
                backoff: ExponentialBackoff {
                    initial_delay: Duration::from_millis(50),
                    max_delay: Duration::from_secs(10),
                    reset_after: Duration::from_secs(60),
                },
                max_restarts: 10,
            },
        });
        supervisor.start();

        sleep(Duration::from_millis(400)).await;
        supervisor.shutdown().await;

        let vec = times.lock().unwrap();
        assert!(vec.len() >= 3, "expected at least 3 attempts, got {}", vec.len());

        let deltas: Vec<Duration> = vec.windows(2).map(|w| w[1].duration_since(w[0])).collect();
        for i in 1..deltas.len() {
            assert!(
                deltas[i] >= deltas[i - 1],
                "backoff did not increase: {:?} vs {:?}",
                deltas[i - 1],
                deltas[i]
            );
        }
    }

    #[tokio::test]
    async fn test_supervisor_max_restarts_exceeded() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter2 = counter.clone();
        let failed = Arc::new(AtomicBool::new(false));
        let failed2 = failed.clone();

        let mut supervisor = Supervisor::new();
        supervisor.on_permanent_failure(move |name: &str| {
            assert_eq!(name, "fail-task");
            failed2.store(true, Ordering::SeqCst);
        });
        supervisor.add_task(SupervisedTask {
            name: "fail-task".to_string(),
            future_factory: Box::new(move || {
                let c = counter2.clone();
                Box::pin(async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "fail")) as BoxError)
                })
            }),
            config: RestartConfig {
                restart_policy: RestartPolicy::OnFailure,
                backoff: ExponentialBackoff {
                    initial_delay: Duration::from_millis(10),
                    max_delay: Duration::from_millis(100),
                    reset_after: Duration::from_secs(60),
                },
                max_restarts: 2,
            },
        });
        supervisor.start();

        sleep(Duration::from_millis(500)).await;
        supervisor.shutdown().await;

        assert!(
            failed.load(Ordering::SeqCst),
            "permanent failure callback should have fired"
        );
        let count = counter.load(Ordering::SeqCst);
        assert_eq!(count, 3, "expected exactly 3 invocations, got {}", count);
    }

    #[tokio::test]
    async fn test_supervisor_graceful_shutdown() {
        let running = Arc::new(AtomicUsize::new(0));
        let running2 = running.clone();

        let mut supervisor = Supervisor::new();
        supervisor.add_task(SupervisedTask {
            name: "loop-task".to_string(),
            future_factory: Box::new(move || {
                let r = running2.clone();
                Box::pin(async move {
                    r.fetch_add(1, Ordering::SeqCst);
                    loop {
                        sleep(Duration::from_millis(50)).await;
                    }
                })
            }),
            config: RestartConfig::default(),
        });
        supervisor.start();

        sleep(Duration::from_millis(150)).await;
        let before = running.load(Ordering::SeqCst);
        assert!(before >= 1, "task should have started");

        supervisor.shutdown().await;

        sleep(Duration::from_millis(100)).await;
        let after = running.load(Ordering::SeqCst);
        assert_eq!(before, after, "task should not have restarted after shutdown");
    }
}
