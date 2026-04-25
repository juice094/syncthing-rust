//! Syncthing 官方 Relay Protocol v1 客户端
//!
//! 与 Go 版 Syncthing 的中继服务器互通，用于 NAT-to-NAT 无法直连时的回退路径。
//!
//! ## 协议要点
//! - Protocol Mode：TLS + `bep-relay` ALPN，用于注册和信令交换
//! - Session Mode：明文 TCP，用于两端数据透传（BEP TLS 在其之上封装）
//!
//! ## 模块结构
//! - `protocol`: XDR 消息定义与编解码 + 异步读写辅助函数
//! - `client`: `RelayProtocolClient`（TLS 信令连接）+ `join_session`（明文会话连接）
//! - `types`: 错误类型与 Result 别名

pub mod protocol;
pub mod client;
pub mod types;
pub mod dial;
pub mod pool;

pub use client::{join_session, RelayProtocolClient};
pub use dial::connect_bep_via_relay;
pub use pool::{fetch_default_relay, fetch_relay_pool, DEFAULT_RELAY_POOL_URL};
pub use protocol::{
    ConnectRequest, Header, JoinRelayRequest, JoinSessionRequest, Message, MessageType, Ping,
    Pong, RelayFull, Response, SessionInvitation, MAGIC,
};
pub use types::{RelayError, Result as RelayResult};
