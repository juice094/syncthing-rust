//! Sync task management.
//!
//! Manages the sync task queue and its execution.

use crate::error::Result;
use std::collections::VecDeque;
use std::sync::Arc;
use syncthing_core::types::FileInfo;
use tokio::sync::Mutex;
use tracing::{debug, trace};

/// A single sync task.
#[derive(Debug, Clone)]
pub struct SyncTask {
    /// Unique task identifier.
    pub id: String,
    /// Folder the task belongs to.
    pub folder: String,
    /// File the task operates on.
    pub file: FileInfo,
    /// What triggered the task.
    pub source: TaskSource,
    /// Scheduling priority.
    pub priority: TaskPriority,
}

/// What triggered a sync task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskSource {
    /// Local change.
    Local,
    /// Remote sync.
    Remote,
    /// Manually triggered.
    Manual,
}

/// Scheduling priority of a sync task (lower value = higher priority).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority {
    /// Critical tasks (e.g. deletions).
    Critical = 0,
    /// High priority.
    High = 1,
    /// Normal priority.
    Normal = 2,
    /// Low priority.
    Low = 3,
    /// Background tasks.
    Background = 4,
}

impl SyncTask {
    /// Creates a new task with normal priority.
    pub fn new(folder: impl Into<String>, file: FileInfo, source: TaskSource) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            folder: folder.into(),
            file,
            source,
            priority: TaskPriority::Normal,
        }
    }

    /// Sets the task priority (builder style).
    pub fn with_priority(mut self, priority: TaskPriority) -> Self {
        self.priority = priority;
        self
    }
}

/// Bounded, priority-ordered task queue.
pub struct TaskQueue {
    queue: Mutex<VecDeque<SyncTask>>,
    max_size: usize,
}

impl TaskQueue {
    /// Creates a new queue holding at most `max_size` tasks.
    pub fn new(max_size: usize) -> Arc<Self> {
        Arc::new(Self {
            queue: Mutex::new(VecDeque::new()),
            max_size,
        })
    }

    /// Adds a task, inserting it ahead of lower-priority tasks.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Other`](crate::SyncError::Other) when the queue is full.
    pub async fn push(&self, task: SyncTask) -> Result<()> {
        let mut queue = self.queue.lock().await;

        if queue.len() >= self.max_size {
            return Err(crate::error::SyncError::Other(
                "Task queue is full".to_string(),
            ));
        }

        let insert_pos = queue
            .iter()
            .position(|t| t.priority > task.priority)
            .unwrap_or(queue.len());

        queue.insert(insert_pos, task);
        debug!(queue_size = queue.len(), "Task added to queue");

        Ok(())
    }

    /// Removes and returns the highest-priority task, if any.
    pub async fn pop(&self) -> Option<SyncTask> {
        let mut queue = self.queue.lock().await;
        let task = queue.pop_front();
        if task.is_some() {
            trace!(queue_size = queue.len(), "Task popped from queue");
        }
        task
    }

    /// Returns the highest-priority task without removing it.
    pub async fn peek(&self) -> Option<SyncTask> {
        let queue = self.queue.lock().await;
        queue.front().cloned()
    }

    /// Returns the number of queued tasks.
    pub async fn len(&self) -> usize {
        self.queue.lock().await.len()
    }

    /// Returns `true` when the queue is empty.
    pub async fn is_empty(&self) -> bool {
        self.queue.lock().await.is_empty()
    }

    /// Removes all tasks from the queue.
    pub async fn clear(&self) {
        let mut queue = self.queue.lock().await;
        queue.clear();
        debug!("Task queue cleared");
    }

    /// Removes and returns all tasks for a specific file.
    pub async fn remove_file(&self, folder: &str, file_name: &str) -> Vec<SyncTask> {
        let mut queue = self.queue.lock().await;
        let mut removed = Vec::new();

        queue.retain(|task| {
            let matches = task.folder == folder && task.file.name == file_name;
            if matches {
                removed.push(task.clone());
            }
            !matches
        });

        removed
    }

    /// Returns all queued tasks for the given folder.
    pub async fn get_folder_tasks(&self, folder: &str) -> Vec<SyncTask> {
        let queue = self.queue.lock().await;
        queue
            .iter()
            .filter(|t| t.folder == folder)
            .cloned()
            .collect()
    }
}

/// Drains a [`TaskQueue`] and runs tasks with bounded concurrency.
pub struct TaskExecutor {
    queue: Arc<TaskQueue>,
    max_concurrent: usize,
}

impl TaskExecutor {
    /// Creates a new executor draining `queue` with at most `max_concurrent`
    /// tasks in flight.
    pub fn new(queue: Arc<TaskQueue>, max_concurrent: usize) -> Self {
        Self {
            queue,
            max_concurrent,
        }
    }

    /// Runs the executor loop until the shutdown watch fires.
    ///
    /// Tasks are popped from the queue and spawned with bounded concurrency;
    /// handler errors are logged and do not stop the loop.
    pub async fn run<F, Fut>(self, mut handler: F, mut shutdown: tokio::sync::watch::Receiver<bool>)
    where
        F: FnMut(SyncTask) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.max_concurrent));

        loop {
            tokio::select! {
                task = async {
                    loop {
                        if let Some(task) = self.queue.pop().await {
                            return Some(task);
                        }
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    }
                } => {
                    if let Some(task) = task {
                        let Ok(permit) = semaphore.clone().acquire_owned().await else {
                            break;
                        };

                        let handler_future = handler(task);

                        tokio::spawn(async move {
                            let _permit = permit;
                            if let Err(e) = handler_future.await {
                                debug!(error = %e, "Task execution failed");
                            }
                        });
                    }
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        debug!("Task executor shutting down");
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syncthing_core::types::{FileInfo, FileType, Vector};

    fn create_test_file(name: &str) -> FileInfo {
        FileInfo {
            name: name.to_string(),
            file_type: FileType::File,
            size: 100,
            permissions: 0o644,
            modified_s: 1234567890,
            modified_ns: 0,
            version: Vector::new(),
            sequence: 1,
            block_size: 128 * 1024,
            blocks: vec![],
            symlink_target: None,
            deleted: None,
            modified_by: None,
            blocks_hash: None,
            no_permissions: None,
            base_version: None,
        }
    }

    #[tokio::test]
    async fn test_task_queue() {
        let queue = TaskQueue::new(100);

        let task = SyncTask::new("test", create_test_file("file.txt"), TaskSource::Local);
        queue.push(task.clone()).await.unwrap();

        assert_eq!(queue.len().await, 1);

        let popped = queue.pop().await;
        assert!(popped.is_some());
        assert_eq!(popped.unwrap().file.name, "file.txt");
    }

    #[tokio::test]
    async fn test_task_priority() {
        let queue = TaskQueue::new(100);

        let low_task = SyncTask::new("test", create_test_file("low.txt"), TaskSource::Local)
            .with_priority(TaskPriority::Low);
        let high_task = SyncTask::new("test", create_test_file("high.txt"), TaskSource::Local)
            .with_priority(TaskPriority::High);

        queue.push(low_task).await.unwrap();
        queue.push(high_task).await.unwrap();

        let first = queue.pop().await.unwrap();
        assert_eq!(first.file.name, "high.txt");
    }

    #[tokio::test]
    async fn test_remove_file() {
        let queue = TaskQueue::new(100);

        let task1 = SyncTask::new("test", create_test_file("file1.txt"), TaskSource::Local);
        let task2 = SyncTask::new("test", create_test_file("file2.txt"), TaskSource::Local);

        queue.push(task1).await.unwrap();
        queue.push(task2).await.unwrap();

        let removed = queue.remove_file("test", "file1.txt").await;
        assert_eq!(removed.len(), 1);
        assert_eq!(queue.len().await, 1);
    }
}
