//! 连接相关类型定义
//!
//! 包括连接状态、地址类型、统计信息、优先级、连接方向、重试配置等。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

/// 连接状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ConnectionState {
    /// 初始状态
    #[default]
    Initial,
    /// 连接中
    Connecting,
    /// 已连接
    Connected,
    /// TLS握手完成
    TlsHandshakeComplete,
    /// 协议握手完成（Hello交换）
    ProtocolHandshakeComplete,
    /// 集群配置交换完成
    ClusterConfigComplete,
    /// 正在断开
    Disconnecting,
    /// 已断开
    Disconnected,
    /// 错误状态
    Error,
}

impl ConnectionState {
    /// 是否处于活跃状态（可用于传输数据）
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            ConnectionState::Connected
                | ConnectionState::TlsHandshakeComplete
                | ConnectionState::ProtocolHandshakeComplete
                | ConnectionState::ClusterConfigComplete
        )
    }

    /// 是否可以发送消息
    pub fn can_send(&self) -> bool {
        matches!(
            self,
            ConnectionState::ProtocolHandshakeComplete | ConnectionState::ClusterConfigComplete
        )
    }

    /// 是否已终止
    pub fn is_terminated(&self) -> bool {
        matches!(self, ConnectionState::Disconnected | ConnectionState::Error)
    }
}

/// 地址类型
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AddressType {
    /// TCP地址
    Tcp(String),
    /// QUIC地址
    Quic(String),
    /// Relay地址
    Relay(String),
    /// 动态发现
    Dynamic,
}

impl AddressType {
    /// 获取地址字符串
    pub fn as_str(&self) -> &str {
        match self {
            AddressType::Tcp(addr) => addr,
            AddressType::Quic(addr) => addr,
            AddressType::Relay(addr) => addr,
            AddressType::Dynamic => "dynamic",
        }
    }
}

impl fmt::Display for AddressType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// 连接统计信息
#[derive(Debug, Clone, Default)]
pub struct ConnectionStats {
    /// 连接建立时间
    pub connected_at: Option<DateTime<Utc>>,
    /// 最后活动时间
    pub last_activity: Option<DateTime<Utc>>,
    /// 发送的字节数
    pub bytes_sent: u64,
    /// 接收的字节数
    pub bytes_received: u64,
    /// 发送的消息数
    pub messages_sent: u64,
    /// 接收的消息数
    pub messages_received: u64,
    /// 重试次数
    pub retry_count: u32,
}

/// 连接优先级
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum ConnectionPriority {
    /// 最低优先级
    Lowest = 0,
    /// 低优先级
    Low = 1,
    /// 正常优先级
    #[default]
    Normal = 2,
    /// 高优先级
    High = 3,
    /// 最高优先级
    Highest = 4,
}

/// 连接类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionType {
    /// 传入连接
    Incoming,
    /// 传出连接（拨号）
    Outgoing,
}

/// 重试策略配置
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// 最大重试次数
    pub max_retries: u32,
    /// 初始退避时间（毫秒）
    pub initial_backoff_ms: u64,
    /// 最大退避时间（毫秒）
    pub max_backoff_ms: u64,
    /// 退避乘数
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 10,
            initial_backoff_ms: 1000,
            max_backoff_ms: 300000, // 5分钟
            backoff_multiplier: 2.0,
        }
    }
}

impl RetryConfig {
    /// 计算第n次重试的退避时间
    pub fn backoff_duration(&self, attempt: u32) -> std::time::Duration {
        if attempt == 0 {
            return std::time::Duration::from_millis(self.initial_backoff_ms);
        }

        let multiplier = self.backoff_multiplier.powi(attempt as i32);
        let backoff_ms = (self.initial_backoff_ms as f64 * multiplier) as u64;
        let backoff_ms = backoff_ms.min(self.max_backoff_ms);

        // 添加抖动（±25%）
        let jitter = rand::random::<f64>() * 0.5 - 0.25;
        let jittered_ms = (backoff_ms as f64 * (1.0 + jitter)) as u64;

        std::time::Duration::from_millis(jittered_ms.max(100))
    }
}
