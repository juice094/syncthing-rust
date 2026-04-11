//! Module: syncthing-sync
//! Worker: Agent-Integration
//! Status: UNVERIFIED
//!
//! ⚠️ 此代码由Agent生成，未经主控验证
//!
//! Accept Loop - 接受传入连接并处理
//!
//! 该模块提供连接接受循环，处理来自远程设备的传入连接。

use std::sync::Arc;

use tracing::{error, info, trace, warn};

use syncthing_core::traits::{BepConnection, ConnectionListener, SyncModel};

/// 接受传入连接并处理
///
/// 在循环中接受连接，并为每个连接启动处理任务
pub async fn accept_loop(
    listener: &mut Box<dyn ConnectionListener>,
    sync_engine: Arc<dyn SyncModel>,
) {
    info!("启动连接接受循环");

    loop {
        match listener.accept().await {
            Ok(Some(conn)) => {
                let device_id = conn.remote_device();
                info!("接受新连接: device={}", device_id.short_id());

                // 启动消息处理任务
                let engine = sync_engine.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(conn, engine).await {
                        warn!("连接处理错误: device={}, error={}", device_id.short_id(), e);
                    }
                });
            }
            Ok(None) => {
                // 超时或暂时无连接，继续循环
                trace!("accept 返回 None，继续等待...");
                continue;
            }
            Err(e) => {
                error!("接受连接错误: {}", e);
                // 短暂等待后重试
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        }
    }
}

/// 处理单个连接
///
/// 1. 交换 Hello（在 TLS 握手时已完成设备 ID 验证）
/// 2. 等待 ClusterConfig
/// 3. 发送本地 Index
/// 4. 消息循环处理
async fn handle_connection(
    conn: Box<dyn BepConnection>,
    sync_engine: Arc<dyn SyncModel>,
) -> syncthing_core::Result<()> {
    let device_id = conn.remote_device();
    info!("开始处理连接: device={}", device_id.short_id());

    // 让同步引擎处理这个连接
    // SyncEngine 实现了 SyncModel trait，会处理消息循环
    sync_engine.handle_connection(conn).await?;

    info!("连接处理完成: device={}", device_id.short_id());
    Ok(())
}

#[cfg(test)]
mod tests {
    use syncthing_core::types::DeviceId;

    #[allow(dead_code)]
    fn test_device() -> DeviceId {
        DeviceId::from_bytes([1u8; 32])
    }

    #[test]
    fn test_message_handler_creation() {
        // 这个测试主要验证编译通过
        // 实际测试需要 mock SyncModel
    }
}
