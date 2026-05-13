use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use syncthing_core::ConnectionType;
use tokio::net::TcpListener;
use tokio::time::sleep;

/// 创建一个占位用的 BepConnection（需要真实 TcpStream）
async fn dummy_bep_connection() -> Arc<BepConnection> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = listener.accept().await;
        sleep(Duration::from_secs(60)).await;
    });
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    BepConnection::new(
        Box::new(crate::connection::TcpBiStream::Plain(stream)),
        ConnectionType::Outgoing,
        tx,
    )
    .await
    .unwrap()
}

struct MockConnector {
    delays: DashMap<SocketAddr, Duration>,
    started: Arc<AtomicUsize>,
    completed: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl DialConnector for MockConnector {
    async fn connect(
        &self,
        addr: SocketAddr,
        _device_id: DeviceId,
        _local_device_id: DeviceId,
        _device_name: &str,
        _tls_config: &Arc<SyncthingTlsConfig>,
    ) -> Result<Arc<BepConnection>, SyncthingError> {
        self.started.fetch_add(1, Ordering::SeqCst);
        if let Some(delay) = self.delays.get(&addr) {
            sleep(*delay).await;
        }
        self.completed.fetch_add(1, Ordering::SeqCst);
        Ok(dummy_bep_connection().await)
    }
}

#[tokio::test]
async fn test_parallel_dialer_race() {
    let local_id = DeviceId::default();
    let mock = Arc::new(MockConnector {
        delays: DashMap::new(),
        started: Arc::new(AtomicUsize::new(0)),
        completed: Arc::new(AtomicUsize::new(0)),
    });

    let fast: SocketAddr = "127.0.0.1:22001".parse().unwrap();
    let medium: SocketAddr = "127.0.0.1:22002".parse().unwrap();
    let slow: SocketAddr = "127.0.0.1:22003".parse().unwrap();

    mock.delays.insert(fast, Duration::from_millis(10));
    mock.delays.insert(medium, Duration::from_millis(50));
    mock.delays.insert(slow, Duration::from_millis(100));

    let dialer = ParallelDialer::new(local_id, "test".to_string(), mock.clone());
    let tls = Arc::new(SyncthingTlsConfig::from_pem(b"", b"").unwrap_or_else(|_| {
        let (cert, key) = crate::tls::generate_certificate("test").unwrap();
        SyncthingTlsConfig::from_pem(&cert, &key).unwrap()
    }));

    let result = dialer
        .dial(
            DeviceId::default(),
            vec![fast, medium, slow],
            vec![],
            &tls,
            &local_id,
        )
        .await;

    assert!(result.is_ok());
    //  fastest wins, but all 3 started
    assert_eq!(mock.started.load(Ordering::SeqCst), 3);
    // 至少 fast 完成；由于 cancel 机制，可能只有 1 个完成
    assert!(mock.completed.load(Ordering::SeqCst) >= 1);
}

#[test]
fn test_address_score_preference() {
    let lan = AddressScore {
        address: "192.168.1.1:22000".parse().unwrap(),
        rtt: Some(Duration::from_millis(50)),
        success_count: 0,
        failure_count: 0,
        last_success: None,
        address_type: AddressTypePreference::Lan,
    };
    let wan = AddressScore {
        address: "8.8.8.8:22000".parse().unwrap(),
        rtt: Some(Duration::from_millis(50)),
        success_count: 0,
        failure_count: 0,
        last_success: None,
        address_type: AddressTypePreference::Wan,
    };

    assert!(
        lan.score() > wan.score(),
        "LAN should score higher than WAN"
    );
}

#[tokio::test]
async fn test_dialer_cancels_slow_connections() {
    let local_id = DeviceId::default();
    let mock = Arc::new(MockConnector {
        delays: DashMap::new(),
        started: Arc::new(AtomicUsize::new(0)),
        completed: Arc::new(AtomicUsize::new(0)),
    });

    let fast: SocketAddr = "127.0.0.1:22004".parse().unwrap();
    let slow: SocketAddr = "127.0.0.1:22005".parse().unwrap();

    mock.delays.insert(fast, Duration::from_millis(10));
    mock.delays.insert(slow, Duration::from_secs(100));

    let dialer = ParallelDialer::new(local_id, "test".to_string(), mock.clone());
    let tls = Arc::new(SyncthingTlsConfig::from_pem(b"", b"").unwrap_or_else(|_| {
        let (cert, key) = crate::tls::generate_certificate("test").unwrap();
        SyncthingTlsConfig::from_pem(&cert, &key).unwrap()
    }));

    let start = Instant::now();
    let result = dialer
        .dial(
            DeviceId::default(),
            vec![fast, slow],
            vec![],
            &tls,
            &local_id,
        )
        .await;
    let elapsed = start.elapsed();

    assert!(result.is_ok());
    // 必须在 fast 的 10ms 附近返回，而不是 slow 的 100s
    assert!(
        elapsed < Duration::from_millis(500),
        "dial should return quickly after fast wins, took {:?}",
        elapsed
    );

    // 给 abort 一点传播时间
    sleep(Duration::from_millis(50)).await;

    // 两者都已启动
    assert_eq!(mock.started.load(Ordering::SeqCst), 2);
    // 慢任务不应该完成
    assert_eq!(
        mock.completed.load(Ordering::SeqCst),
        1,
        "slow connection should have been cancelled"
    );
}
