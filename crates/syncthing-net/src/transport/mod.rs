//! 传输层实现
//!
//! 提供 Transport trait 的具体实现：
//! - `RawTcpTransport`: 原始 TCP（不做 TLS，不做 BEP Hello）
//! - 未来: `WebSocketTransport`, `QuicTransport`, `ProxiedTransport`
//!
//! 设计原则：每个 Transport 只负责建立原始字节管道，
//! TLS/BEP 由上层统一处理。
//!
//! **注意**：`Transport` trait 本身定义在 `syncthing_core::traits` 中，
//! 本模块只提供具体实现和注册表工具。

pub mod tcp;
pub mod bep_adapter;
pub mod proxy;

#[cfg(feature = "websocket")]
pub mod websocket;

use std::sync::Arc;
use syncthing_core::Transport;

/// 传输层注册表，支持多传输共存与最优路径选择。
///
/// 未来扩展：根据 RTT、丢包率、带宽估计等 `PathQuality` 指标动态选择最优传输。
pub struct TransportRegistry {
    transports: Vec<Arc<dyn Transport>>,
}

impl TransportRegistry {
    /// 创建空注册表
    pub fn new() -> Self {
        Self {
            transports: Vec::new(),
        }
    }

    /// 注册一个传输层实现
    pub fn register(&mut self, transport: Arc<dyn Transport>) {
        self.transports.push(transport);
    }

    /// 根据 scheme 查找传输层
    pub fn get(&self, scheme: &str) -> Option<Arc<dyn Transport>> {
        self.transports
            .iter()
            .find(|t| t.scheme() == scheme)
            .cloned()
    }

    /// 返回所有已注册的 scheme 列表
    pub fn schemes(&self) -> Vec<&'static str> {
        self.transports.iter().map(|t| t.scheme()).collect()
    }

    /// 选择默认传输（目前返回第一个注册的）
    pub fn default_transport(&self) -> Option<Arc<dyn Transport>> {
        self.transports.first().cloned()
    }
}

impl Default for TransportRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub use tcp::RawTcpTransport;
