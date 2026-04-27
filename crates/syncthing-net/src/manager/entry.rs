use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use syncthing_core::DeviceId;
use tokio::sync::oneshot;

use crate::connection::BepConnection;

/// 连接条目
#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct ConnectionEntry {
    /// 连接对象
    pub(crate) conn: Arc<BepConnection>,
    /// 连接建立时间
    pub(crate) connected_at: Instant,
    /// 重试次数
    pub(crate) retry_count: u32,
}

impl ConnectionEntry {
    pub(crate) fn new(conn: Arc<BepConnection>) -> Self {
        Self {
            conn,
            connected_at: Instant::now(),
            retry_count: 0,
        }
    }

    pub(crate) fn is_stale(&self, timeout: std::time::Duration) -> bool {
        self.conn
            .last_activity_age()
            .is_none_or(|age| age > timeout)
    }
}

/// 待连接设备
#[allow(dead_code)]
pub(crate) struct PendingConnection {
    pub(crate) device_id: DeviceId,
    pub(crate) addresses: Vec<SocketAddr>,
    pub(crate) relay_urls: Vec<String>,
    pub(crate) retry_count: u32,
    pub(crate) last_attempt: Option<Instant>,
    // 重试任务的取消句柄
    pub(crate) _cancel_tx: Option<oneshot::Sender<()>>,
}
