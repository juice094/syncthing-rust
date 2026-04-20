//! 设备身份抽象
//!
//! `Identity` trait 将设备标识与具体的密码学方案（TLS、WireGuard、PSK 等）解耦。
//! 这是 syncthing-rust 从"文件同步工具"演进为"多传输安全网络层"的基础。
//!
//! 设计原则：
//! - 最小接口：只定义"我是谁"，不涉及握手细节（握手属于传输层）
//! - 零依赖：不引入 tokio_rustls、wireguard 等具体库
//! - 可扩展：具体实现（如 `TlsIdentity`）位于上层 crate（如 `syncthing-net`）

use std::fmt::Debug;
use crate::DeviceId;

/// 设备身份的抽象。
///
/// 不依赖具体的密码学方案。具体实现（如 `TlsIdentity`、`WireGuardIdentity`）
/// 由上层 crate 提供。
///
/// # 设计说明
///
/// `Identity` 只负责"标识"，不负责"握手"。握手逻辑属于传输层：
/// - `TcpTransport` 使用 `TlsIdentity` 做 TLS 握手
/// - `QuicTransport` 使用内置 TLS 1.3 或 `WireGuardIdentity`
/// - `WebSocketTransport` 使用 `TlsIdentity` 做 WSS 握手
///
/// 这种分层让同一设备可以同时拥有多种身份（如 TLS + WireGuard 双栈），
/// 不同传输可以选择最适合的身份方案。
pub trait Identity: Send + Sync + Debug {
    /// 获取本设备的唯一标识符。
    ///
    /// 对于所有实现，`device_id` 都应该是 32 字节的固定长度标识符，
    /// 与底层密码学方案无关。
    fn device_id(&self) -> DeviceId;

    /// 身份方案名称。
    ///
    /// 示例值：`"tls"`、`"wireguard"`、`"psk"`、`"noise"`
    fn scheme(&self) -> &'static str;
}

/// 最简单的身份实现：直接持有 DeviceId，无额外密码学材料。
///
/// 用于测试、内存管道场景，或调用方尚未迁移到具体身份方案时的过渡。
#[derive(Debug, Clone)]
pub struct DeviceIdentity {
    id: DeviceId,
}

impl DeviceIdentity {
    pub fn new(id: DeviceId) -> Self {
        Self { id }
    }
}

impl Identity for DeviceIdentity {
    fn device_id(&self) -> DeviceId {
        self.id
    }

    fn scheme(&self) -> &'static str {
        "device"
    }
}

/// 为 `Arc<dyn Identity>` 提供便捷的 `device_id` 访问。
impl Identity for std::sync::Arc<dyn Identity> {
    fn device_id(&self) -> DeviceId {
        (**self).device_id()
    }

    fn scheme(&self) -> &'static str {
        (**self).scheme()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DeviceId;

    #[derive(Debug)]
    struct MockIdentity {
        id: DeviceId,
    }

    impl Identity for MockIdentity {
        fn device_id(&self) -> DeviceId {
            self.id
        }

        fn scheme(&self) -> &'static str {
            "mock"
        }
    }

    #[test]
    fn test_identity_trait() {
        let id = DeviceId::default();
        let identity = MockIdentity { id };
        assert_eq!(identity.device_id(), id);
        assert_eq!(identity.scheme(), "mock");
    }

    #[test]
    fn test_arc_identity() {
        let id = DeviceId::default();
        let identity: std::sync::Arc<dyn Identity> = std::sync::Arc::new(MockIdentity { id });
        assert_eq!(identity.device_id(), id);
        assert_eq!(identity.scheme(), "mock");
    }
}
