use std::net::SocketAddr;
use std::sync::Arc;

use crate::tcp_transport::DEFAULT_TCP_PORT;

use super::*;

    #[test]
    fn test_connection_manager_config_default() {
        let config = ConnectionManagerConfig::default();
        assert_eq!(config.listen_addr.port(), DEFAULT_TCP_PORT);
        assert_eq!(config.max_connections, 1000);
    }

    #[tokio::test]
    async fn test_rebind_triggers_redial() {
        let tls_config = Arc::new(SyncthingTlsConfig::from_pem(b"", b"").unwrap_or_else(|_| {
            let (cert, key) = crate::tls::generate_certificate("syncthing-rust-test")
                .expect("failed to generate certificate");
            SyncthingTlsConfig::from_pem(&cert, &key).expect("failed to load generated certificate")
        }));
        let identity = Arc::new(crate::identity::TlsIdentity::new(Arc::clone(&tls_config)));
        let (manager, _handle) =
            ConnectionManager::new(ConnectionManagerConfig::default(), identity, tls_config);

        // Register a device address but don't connect
        let device_id = DeviceId::default();
        let addr: SocketAddr = "127.0.0.1:22001".parse().unwrap();
        manager.device_addresses.insert(device_id, vec![addr]);

        // Manually trigger network change handling
        manager
            .handle_net_change(NetChangeEvent::InterfacesChanged)
            .await;

        // Verify pending connection was created
        let pending = manager.pending_connections.read().await;
        assert!(pending.contains_key(&device_id));
    }

    #[tokio::test]
    async fn test_transport_registry_start_listen() {
        // Phase 2 验证：ConnectionManager 通过 TransportRegistry 启动监听
        let tls_config = Arc::new(SyncthingTlsConfig::from_pem(b"", b"").unwrap_or_else(|_| {
            let (cert, key) = crate::tls::generate_certificate("transport-registry-test")
                .expect("failed to generate certificate");
            SyncthingTlsConfig::from_pem(&cert, &key).expect("failed to load generated certificate")
        }));
        let identity = Arc::new(crate::identity::TlsIdentity::new(Arc::clone(&tls_config)));
        let config = ConnectionManagerConfig {
            listen_addr: "127.0.0.1:0".parse().unwrap(),
            ..Default::default()
        };
        let (manager, _handle) = ConnectionManager::new(config, identity, tls_config);

        // 注册 TransportRegistry
        let mut registry = crate::transport::TransportRegistry::new();
        registry.register(Arc::new(crate::transport::RawTcpTransport::new()));
        manager.set_transport_registry(Arc::new(registry));

        // 启动监听
        let addr = manager
            .start()
            .await
            .expect("failed to start with TransportRegistry");
        assert!(addr.port() > 0, "should bind to a random port");

        // 清理
        manager.stop().await.expect("failed to stop");
    }

    #[test]
    fn test_should_reconnect_reasons() {
        let tls_config = Arc::new(SyncthingTlsConfig::from_pem(b"", b"").unwrap_or_else(|_| {
            let (cert, key) = crate::tls::generate_certificate("reconnect-test")
                .expect("failed to generate certificate");
            SyncthingTlsConfig::from_pem(&cert, &key).expect("failed to load generated certificate")
        }));
        let identity = Arc::new(crate::identity::TlsIdentity::new(Arc::clone(&tls_config)));
        let (manager, _handle) =
            ConnectionManager::new(ConnectionManagerConfig::default(), identity, tls_config);

        let device_id = DeviceId::default();

        // 不应重连的情况
        assert!(!manager.should_reconnect(&device_id, "manual disconnect"));
        assert!(!manager.should_reconnect(&device_id, "invalid device ID"));
        assert!(!manager.should_reconnect(&device_id, "unauthorized"));
        assert!(!manager.should_reconnect(&device_id, "paused by user"));

        // 应该重连的情况
        assert!(manager.should_reconnect(&device_id, "connection reset"));
        assert!(manager.should_reconnect(&device_id, "timed out"));
        assert!(manager.should_reconnect(&device_id, ""));
    }
