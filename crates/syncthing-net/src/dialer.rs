//! 并行拨号器
//!
//! 实现多地址并发拨号、地址质量评分和最优连接选择

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use syncthing_core::{DeviceId, SyncthingError};

use crate::connection::{BepConnection, TcpBiStream};
use crate::tcp_transport::connect_bep;
use crate::tls::SyncthingTlsConfig;

/// 地址类型偏好（影响评分排序）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AddressTypePreference {
    /// 中继地址（优先级最低）
    Relay,
    /// 公网地址
    Wan,
    /// 局域网地址（优先级最高）
    Lan,
}

/// 地址评分记录
#[derive(Debug, Clone)]
pub struct AddressScore {
    /// 目标地址
    pub address: SocketAddr,
    /// 最近握手RTT
    pub rtt: Option<Duration>,
    /// 成功次数
    pub success_count: u32,
    /// 失败次数
    pub failure_count: u32,
    /// 上次成功时间
    pub last_success: Option<Instant>,
    /// 地址类型偏好
    pub address_type: AddressTypePreference,
}

impl AddressScore {
    /// 计算该地址的当前得分
    ///
    /// 得分规则：
    /// - LAN 基础分 > WAN 基础分 > Relay 基础分
    /// - RTT 越低加分越多（上限 400ms）
    /// - 每次成功 +10_000
    /// - 最近 1 小时内有成功额外加分
    /// - 每次失败 -50_000
    pub fn score(&self) -> u64 {
        let mut score: u64 = 0;

        // 类型基础分
        match self.address_type {
            AddressTypePreference::Lan => score += 1_000_000,
            AddressTypePreference::Wan => score += 500_000,
            AddressTypePreference::Relay => score += 100_000,
        }

        // RTT 奖励：越低越好
        if let Some(rtt) = self.rtt {
            let rtt_ms = rtt.as_millis() as u64;
            let rtt_bonus = if rtt_ms < 400 { 400 - rtt_ms } else { 0 };
            score += rtt_bonus * 100;
        }

        // 成功次数奖励
        score += self.success_count as u64 * 10_000;

        // 最近成功奖励（1 小时内递减）
        if let Some(last) = self.last_success {
            let elapsed_secs = last.elapsed().as_secs();
            if elapsed_secs < 3600 {
                score += (3600 - elapsed_secs) * 10;
            }
        }

        // 失败惩罚
        score = score.saturating_sub(self.failure_count as u64 * 50_000);

        score
    }
}

/// 拨号连接器抽象（便于测试替换）
#[async_trait::async_trait]
pub trait DialConnector: Send + Sync {
    /// 对单个地址执行 TCP + BEP 握手
    async fn connect(
        &self,
        addr: SocketAddr,
        device_id: DeviceId,
        local_device_id: DeviceId,
        device_name: &str,
        tls_config: &Arc<SyncthingTlsConfig>,
    ) -> Result<Arc<BepConnection>, SyncthingError>;
}

/// 基于真实 TCP 传输的连接器
pub struct TcpBepConnector;

#[async_trait::async_trait]
impl DialConnector for TcpBepConnector {
    async fn connect(
        &self,
        addr: SocketAddr,
        device_id: DeviceId,
        local_device_id: DeviceId,
        device_name: &str,
        tls_config: &Arc<SyncthingTlsConfig>,
    ) -> Result<Arc<BepConnection>, SyncthingError> {
        connect_bep(addr, device_id, local_device_id, device_name, tls_config).await
    }
}

/// 并行拨号器
///
/// 维护每地址的历史评分，支持对多个候选地址并发拨号并返回最先成功的连接。
pub struct ParallelDialer {
    /// 每地址评分表
    scores: DashMap<SocketAddr, AddressScore>,
    /// 每设备地址评分（冗余备份，便于管理器快速查询）
    device_scores: DashMap<DeviceId, Vec<AddressScore>>,
    /// 本地设备ID
    local_device_id: DeviceId,
    /// 设备名称（用于 Hello）
    device_name: String,
    /// 底层连接器
    connector: Arc<dyn DialConnector>,
}

impl ParallelDialer {
    /// 使用自定义连接器创建
    pub fn new(
        local_device_id: DeviceId,
        device_name: String,
        connector: Arc<dyn DialConnector>,
    ) -> Self {
        Self {
            scores: DashMap::new(),
            device_scores: DashMap::new(),
            local_device_id,
            device_name,
            connector,
        }
    }

    /// 使用默认 TCP 连接器创建
    pub fn with_tcp_connector(local_device_id: DeviceId, device_name: String) -> Self {
        Self::new(local_device_id, device_name, Arc::new(TcpBepConnector))
    }

    /// 获取或初始化某地址的评分记录
    pub fn get_or_create_score(&self, addr: SocketAddr) -> AddressScore {
        self.scores
            .entry(addr)
            .or_insert_with(|| AddressScore {
                address: addr,
                rtt: None,
                success_count: 0,
                failure_count: 0,
                last_success: None,
                address_type: infer_address_type(addr),
            })
            .clone()
    }

    /// 记录某地址拨号成功
    pub fn record_success(&self, addr: SocketAddr, rtt: Duration) {
        if let Some(mut score) = self.scores.get_mut(&addr) {
            score.rtt = Some(rtt);
            score.success_count += 1;
            score.last_success = Some(Instant::now());
        }
    }

    /// 记录某地址拨号失败
    pub fn record_failure(&self, addr: SocketAddr) {
        if let Some(mut score) = self.scores.get_mut(&addr) {
            score.failure_count += 1;
        }
    }

    /// 并发拨号
    ///
    /// 1. 按历史评分对地址排序
    /// 2. 取前 3 个地址并发拨号
    /// 3. 第一个成功握手者胜出，其余任务立即取消
    /// 4. 更新该地址的评分统计
    pub async fn dial(
        &self,
        device_id: DeviceId,
        addresses: Vec<SocketAddr>,
        tls_config: &Arc<SyncthingTlsConfig>,
        _local_device_id: &DeviceId,
    ) -> Result<Arc<BepConnection>, SyncthingError> {
        if addresses.is_empty() {
            return Err(SyncthingError::connection("no addresses to dial"));
        }

        // 构造评分并降序排序
        let mut scored: Vec<AddressScore> = addresses
            .iter()
            .map(|addr| self.get_or_create_score(*addr))
            .collect();
        scored.sort_by(|a, b| b.score().cmp(&a.score()));

        // 最多并发 3 个
        let top: Vec<AddressScore> = scored.into_iter().take(3).collect();

        info!(
            "Parallel dialing {} with {} candidates (top 3: {:?})",
            device_id,
            addresses.len(),
            top.iter().map(|s| (s.address, s.score())).collect::<Vec<_>>()
        );

        // 启动并发拨号任务
        let mut tasks: FuturesUnordered<
            JoinHandle<Result<(Arc<BepConnection>, SocketAddr, Duration), SyncthingError>>,
        > = FuturesUnordered::new();

        for score in &top {
            let addr = score.address;
            let connector = Arc::clone(&self.connector);
            let device_id = device_id;
            let local_device_id = self.local_device_id;
            let device_name = self.device_name.clone();
            let tls_config = Arc::clone(tls_config);

            let handle: JoinHandle<
                Result<(Arc<BepConnection>, SocketAddr, Duration), SyncthingError>,
            > = tokio::spawn(async move {
                let start = Instant::now();
                match connector.connect(addr, device_id, local_device_id, &device_name, &tls_config).await {
                    Ok(conn) => {
                        let rtt = start.elapsed();
                        debug!("Dial to {} succeeded in {:?}", addr, rtt);
                        Ok((conn, addr, rtt))
                    }
                    Err(e) => {
                        debug!("Dial to {} failed: {}", addr, e);
                        Err(e)
                    }
                }
            });

            tasks.push(handle);
        }

        let mut last_error: Option<SyncthingError> = None;
        while let Some(result) = tasks.next().await {
            match result {
                Ok(Ok((conn, addr, rtt))) => {
                    // 成功：取消剩余任务
                    for task in tasks {
                        task.abort();
                    }
                    self.record_success(addr, rtt);
                    return Ok(conn);
                }
                Ok(Err(e)) => {
                    last_error = Some(e);
                }
                Err(e) => {
                    last_error = Some(SyncthingError::connection(format!(
                        "dial task panicked: {}",
                        e
                    )));
                }
            }
        }

        // 全部失败：为每个参与的地址记录失败（若尚未记录）
        for score in &top {
            self.record_failure(score.address);
        }

        Err(last_error.unwrap_or_else(|| {
            SyncthingError::connection("all parallel dial attempts failed")
        }))
    }

    /// 获取内部评分表引用（供管理器查询）
    pub fn address_scores(&self) -> &DashMap<SocketAddr, AddressScore> {
        &self.scores
    }

    /// 获取某设备的所有地址评分
    pub fn device_address_scores(&self, device_id: &DeviceId) -> Option<Vec<AddressScore>> {
        self.device_scores.get(device_id).map(|e| e.clone())
    }

    /// 批量更新某设备的地址评分缓存
    pub fn update_device_scores(&self, device_id: DeviceId, scores: Vec<AddressScore>) {
        self.device_scores.insert(device_id, scores);
    }
}

/// 根据 IP 特征推断地址类型偏好
fn infer_address_type(addr: SocketAddr) -> AddressTypePreference {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    let ip = addr.ip();
    let is_private = match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unicast_link_local(),
    };
    let is_multicast = match ip {
        IpAddr::V4(v4) => v4.is_multicast(),
        IpAddr::V6(v6) => v6.is_multicast(),
    };

    if is_private {
        AddressTypePreference::Lan
    } else if is_multicast {
        AddressTypePreference::Relay
    } else {
        AddressTypePreference::Wan
    }
}

#[cfg(test)]
mod tests {
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
        BepConnection::new(Box::new(crate::connection::TcpBiStream::Plain(stream)), ConnectionType::Outgoing, tx)
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
        let tls = Arc::new(
            SyncthingTlsConfig::from_pem(b"", b"").unwrap_or_else(|_| {
                let (cert, key) = crate::tls::generate_certificate("test").unwrap();
                SyncthingTlsConfig::from_pem(&cert, &key).unwrap()
            }),
        );

        let result = dialer
            .dial(DeviceId::default(), vec![fast, medium, slow], &tls, &local_id)
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

        assert!(lan.score() > wan.score(), "LAN should score higher than WAN");
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
        let tls = Arc::new(
            SyncthingTlsConfig::from_pem(b"", b"").unwrap_or_else(|_| {
                let (cert, key) = crate::tls::generate_certificate("test").unwrap();
                SyncthingTlsConfig::from_pem(&cert, &key).unwrap()
            }),
        );

        let start = Instant::now();
        let result = dialer
            .dial(DeviceId::default(), vec![fast, slow], &tls, &local_id)
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
}
