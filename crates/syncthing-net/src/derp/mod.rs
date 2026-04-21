//! DERP（Detoured Encrypted Routing Protocol）中继协议
//!
//! 极简的 P2P 中继协议，用于 NAT 穿透失败时的数据包转发。
//!
//! ## 核心设计
//! - 帧格式：`[4 bytes len] [1 byte type] [payload]`
//! - 客户端注册后，服务器按 device_id 转发数据包
//! - 支持 TCP 和 WebSocket 传输（WebSocket 可伪装为 HTTPS 流量）
//! - 支持 Relay 链式转发（Client → Relay A → Relay B → Peer）
//!
//! ## 模块结构
//! - `protocol`: 帧格式定义与编解码
//! - `client`: DERP 客户端（连接 relay，发送/接收数据包）
//! - `server`: DERP 服务器（接受连接，按 device_id 转发）

pub mod protocol;
pub mod client;
pub mod server;
pub mod pipe;
pub mod transport;

pub use protocol::{Frame, FrameType, PROTOCOL_VERSION, MAX_FRAME_SIZE};
pub use client::{DerpClient, DerpClientConfig, DerpClientState};
pub use server::{DerpServer, DerpServerConfig};
pub use pipe::DerpPipe;
pub use transport::DerpTransport;
