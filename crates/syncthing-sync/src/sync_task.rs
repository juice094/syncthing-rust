//! 同步任务管理
//! 
//! 管理同步任务队列和执行

use crate::error::Result;
use syncthing_core::DeviceId;
use syncthing_core::types::FileInfo;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, trace};

/// 同步任务
#[derive(Debug, Clone)]
pub struct SyncTask {
    pub id: String,
    pub folder: String,
    pub file: FileInfo,
    pub source: TaskSource,
    pub priority: TaskPriority,
}

/// 任务来源
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskSource {
    Local,      // 本地变更
    Remote,     // 远程同步
    Manual,     // 手动触发
}

/// 任务优先级
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority {
    Critical = 0,  // 关键任务（如删除操作）
    High = 1,      // 高优先级
    Normal = 2,    // 普通优先级
    Low = 3,       // 低优先级
    Background = 4,// 后台任务
}

impl SyncTask {
    /// 创建新任务
    pub fn new(folder: impl Into<String>, file: FileInfo, source: TaskSource) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            folder: folder.into(),
            file,
            source,
            priority: TaskPriority::Normal,
        }
    }

    /// 设置优先级
    pub fn with_priority(mut self, priority: TaskPriority) -> Self {
        self.priority = priority;
        self
    }
}

/// 任务队列
pub struct TaskQueue {
    queue: Mutex<VecDeque<SyncTask>>,
    max_size: usize,
}

impl TaskQueue {
    /// 创建新的任务队列
    pub fn new(max_size: usize) -> Arc<Self> {
        Arc::new(Self {
            queue: Mutex::new(VecDeque::new()),
            max_size,
        })
    }

    /// 添加任务
    pub async fn push(&self, task: SyncTask) -> Result<()> {
        let mut queue = self.queue.lock().await;
        
        if queue.len() >= self.max_size {
            return Err(crate::error::SyncError::Other("Task queue is full".to_string()));
        }

        // 按优先级插入
        let insert_pos = queue.iter()
            .position(|t| t.priority > task.priority)
            .unwrap_or(queue.len());
        
        queue.insert(insert_pos, task);
        debug!(queue_size = queue.len(), "Task added to queue");
        
        Ok(())
    }

    /// 获取下一个任务
    pub async fn pop(&self) -> Option<SyncTask> {
        let mut queue = self.queue.lock().await;
        let task = queue.pop_front();
        if task.is_some() {
            trace!(queue_size = queue.len(), "Task popped from queue");
        }
        task
    }

    /// 查看下一个任务（不删除）
    pub async fn peek(&self) -> Option<SyncTask> {
        let queue = self.queue.lock().await;
        queue.front().cloned()
    }

    /// 获取队列大小
    pub async fn len(&self) -> usize {
        self.queue.lock().await.len()
    }

    /// 检查队列是否为空
    pub async fn is_empty(&self) -> bool {
        self.queue.lock().await.is_empty()
    }

    /// 清空队列
    pub async fn clear(&self) {
        let mut queue = self.queue.lock().await;
        queue.clear();
        debug!("Task queue cleared");
    }

    /// 移除特定文件的任务
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

    /// 获取指定文件夹的所有任务
    pub async fn get_folder_tasks(&self, folder: &str) -> Vec<SyncTask> {
        let queue = self.queue.lock().await;
        queue.iter()
            .filter(|t| t.folder == folder)
            .cloned()
            .collect()
    }
}

/// 任务执行器
pub struct TaskExecutor {
    queue: Arc<TaskQueue>,
    max_concurrent: usize,
}

impl TaskExecutor {
    /// 创建新的任务执行器
    pub fn new(queue: Arc<TaskQueue>, max_concurrent: usize) -> Self {
        Self {
            queue,
            max_concurrent,
        }
    }

    /// 启动执行器
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
                        let permit = match semaphore.clone().acquire_owned().await {
                            Ok(p) => p,
                            Err(_) => break,
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
