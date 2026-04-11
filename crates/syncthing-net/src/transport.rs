//! Iroh 传输实现
//!
//! 使用 iroh Endpoint 作为现代 P2P 传输层后端。
//! Router 集成暂时标记为 TODO，当前使用 endpoint.accept() 裸循环。

use std::sync::Arc;
use tracing::{debug, info, warn};

use iroh::Endpoint;
use iroh::endpoint::presets;

use syncthing_core::{DeviceId, Result, SyncthingError};

use crate::connection::IrohBepConnection;
use crate::connection::BEP_ALPN;
use crate::tls::SyncthingTlsConfig;

/// Iroh 传输层
pub struct IrohTransport {
    endpoint: Endpoint,
    local_device_id: DeviceId,
    device_name: String,
    client_name: String,
    client_version: String,
    tls_config: Arc<SyncthingTlsConfig>,
}

impl IrohTransport {
    /// 创建并绑定新的 iroh 传输层
    pub async fn new(
        local_device_id: DeviceId,
        device_name: String,
        tls_config: Arc<SyncthingTlsConfig>,
    ) -> Result<Self> {
        let endpoint = Endpoint::builder(presets::N0)
            .alpns(vec![BEP_ALPN.to_vec()])
            .bind()
            .await
            .map_err(|e| SyncthingError::connection(format!("iroh endpoint bind failed: {}", e)))?;

        info!("Iroh endpoint bound, node_id={}", endpoint.id());

        Ok(Self {
            endpoint,
            local_device_id,
            device_name,
            client_name: "syncthing-rust".to_string(),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
            tls_config,
        })
    }

    /// 获取底层 Endpoint
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// 获取本端节点地址
    pub fn node_addr(&self) -> iroh::EndpointAddr {
        self.endpoint.addr()
    }

    /// 连接到指定地址
    pub async fn connect(&self, addr: iroh::EndpointAddr) -> Result<Arc<IrohBepConnection>> {
        debug!("Connecting via iroh to {:?}", addr);
        let conn = self
            .endpoint
            .connect(addr, BEP_ALPN)
            .await
            .map_err(|e| SyncthingError::connection(format!("iroh connect failed: {}", e)))?;

        let (bep_conn, _remote_hello) = IrohBepConnection::connect(
            conn,
            &self.tls_config,
            &self.device_name,
            &self.client_name,
            &self.client_version,
        )
        .await?;

        bep_conn.set_device_id(self.local_device_id);
        Ok(bep_conn)
    }

    /// 启动监听循环（裸 accept 循环，Router 集成标记为 TODO）
    pub async fn listen(&self) -> Result<()> {
        info!("Iroh listener starting on node_id={}", self.endpoint.id());
        let tls_config = Arc::clone(&self.tls_config);
        let device_name = self.device_name.clone();
        let client_name = self.client_name.clone();
        let client_version = self.client_version.clone();

        loop {
            let Some(incoming) = self.endpoint.accept().await else {
                info!("Iroh endpoint closed, stopping listener");
                break;
            };

            let conn = match incoming.await {
                Ok(conn) => conn,
                Err(e) => {
                    warn!("Iroh incoming connection failed: {}", e);
                    continue;
                }
            };

            let tls_config = Arc::clone(&tls_config);
            let device_name = device_name.clone();
            let client_name = client_name.clone();
            let client_version = client_version.clone();

            tokio::spawn(async move {
                match IrohBepConnection::accept(
                    conn,
                    &tls_config,
                    &device_name,
                    &client_name,
                    &client_version,
                )
                .await
                {
                    Ok((bep_conn, remote_hello)) => {
                        info!(
                            "Accepted iroh BEP connection from {}",
                            remote_hello.device_name
                        );
                        // TODO: 通过 Router/ProtocolHandler 注册到 ConnectionManager
                        // 目前仅保持连接存活，直到对端关闭
                        let _ = bep_conn;
                    }
                    Err(e) => {
                        warn!("Failed to accept iroh BEP connection: {}", e);
                    }
                }
            });
        }
        Ok(())
    }

    /// 优雅关闭 endpoint
    pub async fn close(&self) {
        self.endpoint.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::IrohBepConnection;
    use crate::protocol::MessageType;
    use iroh::endpoint::presets;

    #[tokio::test]
    async fn test_iroh_bep_hello_and_ping() {
        let _ = tracing_subscriber::fmt::try_init();

        let (cert1, key1) = crate::tls::generate_certificate("test1").unwrap();
        let tls1 = Arc::new(SyncthingTlsConfig::from_pem(&cert1, &key1).unwrap());

        let (cert2, key2) = crate::tls::generate_certificate("test2").unwrap();
        let tls2 = Arc::new(SyncthingTlsConfig::from_pem(&cert2, &key2).unwrap());

        let ep1 = Endpoint::builder(presets::Minimal)
            .clear_ip_transports()
            .bind_addr("127.0.0.1:0")
            .expect("bind_addr ep1")
            .alpns(vec![BEP_ALPN.to_vec()])
            .bind()
            .await
            .expect("bind ep1");
        let ep2 = Endpoint::builder(presets::Minimal)
            .clear_ip_transports()
            .bind_addr("127.0.0.1:0")
            .expect("bind_addr ep2")
            .alpns(vec![BEP_ALPN.to_vec()])
            .bind()
            .await
            .expect("bind ep2");

        let addr1 = ep1.addr();

        // Clone endpoint so it stays alive in the main task
        let ep1_clone = ep1.clone();
        let tls1_clone = Arc::clone(&tls1);
        let accept_task = tokio::spawn(async move {
            let incoming = ep1_clone.accept().await.expect("accept incoming");
            let conn = incoming.await.expect("incoming into future");
            IrohBepConnection::accept(conn, &tls1_clone, "listener", "test", "0.1.0").await
        });

        let conn2 = ep2.connect(addr1, BEP_ALPN).await.expect("connect");
        let (client_conn, client_hello) = IrohBepConnection::connect(
            conn2,
            &tls2,
            "dialer",
            "test",
            "0.1.0",
        )
        .await
        .expect("client connect");

        let (server_conn, server_hello) = accept_task
            .await
            .expect("accept task join")
            .expect("server accept");

        assert_eq!(client_hello.device_name, "listener");
        assert_eq!(server_hello.device_name, "dialer");
        assert!(client_conn.is_hello_complete());
        assert!(server_conn.is_hello_complete());

        // Exchange Ping/Ping (BEP has no Pong)
        client_conn.send_ping().await.expect("send ping");
        let (msg_type, payload) = server_conn.recv_message().await.expect("recv message");
        assert_eq!(msg_type, MessageType::Ping);
        assert!(payload.is_empty());

        server_conn.send_ping().await.expect("send ping reply");
        let (msg_type2, payload2) = client_conn.recv_message().await.expect("recv message");
        assert_eq!(msg_type2, MessageType::Ping);
        assert!(payload2.is_empty());

        // Gracefully close endpoints to avoid ERROR log
        ep1.close().await;
        ep2.close().await;
    }
}
