//! 并行拨号器
//!
//! 实现多地址并发拨号、地址质量评分和最优连接选择

use std::cmp::Reverse;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use parking_lot::RwLock;
use tokio::task::JoinHandle;
use tracing::{debug, info};

use syncthing_core::{DeviceId, SyncthingError};

use crate::connection::BepConnection;
use crate::relay::dial::parse_relay_url;
use crate::tcp_transport::connect_bep;
use crate::tls::SyncthingTlsConfig;

/// Result type for a dial task.
pub type DialResult = Result<(Arc<BepConnection>, SocketAddr, Duration), SyncthingError>;

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
            let rtt_bonus = 400_u64.saturating_sub(rtt_ms);
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

/// Relay 拨号连接器抽象
#[async_trait::async_trait]
pub trait RelayDialConnector: Send + Sync {
    /// 通过 relay 服务器建立 BEP 连接
    async fn connect_via_relay(
        &self,
        relay_url: &str,
        target_device_id: DeviceId,
        local_device_id: DeviceId,
        device_name: &str,
        tls_config: &Arc<SyncthingTlsConfig>,
    ) -> Result<Arc<BepConnection>, SyncthingError>;
}

/// 基于真实 Relay 协议的连接器
pub struct RelayBepConnector;

#[async_trait::async_trait]
impl RelayDialConnector for RelayBepConnector {
    async fn connect_via_relay(
        &self,
        relay_url: &str,
        target_device_id: DeviceId,
        local_device_id: DeviceId,
        device_name: &str,
        tls_config: &Arc<SyncthingTlsConfig>,
    ) -> Result<Arc<BepConnection>, SyncthingError> {
        // 若本端 device ID 较小，在 relay 握手前短暂退让。
        crate::manager::handshake::pre_handshake_yield(local_device_id, target_device_id).await;

        crate::relay::connect_bep_via_relay(relay_url, target_device_id, device_name, tls_config)
            .await
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
    /// 默认连接器（向后兼容）
    connector: RwLock<Arc<dyn DialConnector>>,
    /// Scheme → 连接器映射（支持多传输路由）
    connectors: DashMap<String, Arc<dyn DialConnector>>,
    /// Relay 连接器（可选）
    relay_connector: Option<Arc<dyn RelayDialConnector>>,
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
            connector: RwLock::new(connector),
            connectors: DashMap::new(),
            relay_connector: None,
        }
    }

    /// 使用默认 TCP 连接器创建
    pub fn with_tcp_connector(local_device_id: DeviceId, device_name: String) -> Self {
        let mut dialer = Self::new(local_device_id, device_name, Arc::new(TcpBepConnector));
        dialer.relay_connector = Some(Arc::new(RelayBepConnector));
        dialer
    }

    /// 设置 Relay 连接器
    pub fn set_relay_connector(&mut self, connector: Arc<dyn RelayDialConnector>) {
        self.relay_connector = Some(connector);
    }

    /// 更换底层连接器（用于 Transport 注册后切换）
    pub fn set_connector(&self, connector: Arc<dyn DialConnector>) {
        *self.connector.write() = connector;
    }

    /// 注册 scheme-specific 连接器
    pub fn register_connector(&self, scheme: impl Into<String>, connector: Arc<dyn DialConnector>) {
        let scheme = scheme.into();
        debug!("Registering dial connector for scheme: {}", scheme);
        self.connectors.insert(scheme, connector);
    }

    /// 获取已注册的 scheme 列表
    pub fn registered_schemes(&self) -> Vec<String> {
        self.connectors.iter().map(|e| e.key().clone()).collect()
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

    /// 并发拨号（向后兼容包装）
    ///
    /// 调用 `dial_with_schemes` 并传入 `None` 作为 scheme 列表，
    /// 所有 direct 地址使用默认连接器。
    pub async fn dial(
        &self,
        device_id: DeviceId,
        addresses: Vec<SocketAddr>,
        relay_urls: Vec<String>,
        tls_config: &Arc<SyncthingTlsConfig>,
        local_device_id: &DeviceId,
    ) -> Result<Arc<BepConnection>, SyncthingError> {
        self.dial_with_schemes(
            device_id,
            addresses,
            None,
            relay_urls,
            tls_config,
            local_device_id,
        )
        .await
    }

    /// 并发拨号（支持 scheme-aware 路由）
    ///
    /// 1. 按历史评分对地址排序（direct 与 relay 共同参与排序）
    /// 2. 取前 3 个候选并发拨号
    /// 3. 第一个成功握手者胜出，其余任务立即取消
    /// 4. 更新该地址的评分统计
    pub async fn dial_with_schemes(
        &self,
        device_id: DeviceId,
        addresses: Vec<SocketAddr>,
        address_schemes: Option<Vec<String>>,
        relay_urls: Vec<String>,
        tls_config: &Arc<SyncthingTlsConfig>,
        _local_device_id: &DeviceId,
    ) -> Result<Arc<BepConnection>, SyncthingError> {
        if addresses.is_empty() && relay_urls.is_empty() {
            return Err(SyncthingError::connection("no addresses to dial"));
        }

        // 构造候选列表：(is_relay, socket_addr, optional_relay_url, score, scheme)
        let mut candidates: Vec<(bool, SocketAddr, Option<String>, AddressScore, String)> =
            Vec::new();

        // Direct candidates
        for (i, addr) in addresses.iter().enumerate() {
            let scheme = address_schemes
                .as_ref()
                .and_then(|v| v.get(i).cloned())
                .unwrap_or_else(|| "tcp".to_string());
            candidates.push((false, *addr, None, self.get_or_create_score(*addr), scheme));
        }

        // Relay candidates
        for url in &relay_urls {
            if let Ok((relay_addr, _)) = parse_relay_url(url) {
                let score = self
                    .scores
                    .entry(relay_addr)
                    .or_insert_with(|| AddressScore {
                        address: relay_addr,
                        rtt: None,
                        success_count: 0,
                        failure_count: 0,
                        last_success: None,
                        address_type: AddressTypePreference::Relay,
                    })
                    .clone();
                candidates.push((
                    true,
                    relay_addr,
                    Some(url.clone()),
                    score,
                    "relay".to_string(),
                ));
            }
        }

        // 按评分降序排序
        candidates.sort_by_key(|(_, _, _, s, _)| Reverse(s.score()));

        // 最多并发 3 个
        let top: Vec<(bool, SocketAddr, Option<String>, AddressScore, String)> =
            candidates.into_iter().take(3).collect();

        info!(
            "Parallel dialing {} with {} direct + {} relay candidates (top 3: {:?})",
            device_id,
            addresses.len(),
            relay_urls.len(),
            top.iter()
                .map(|(_, addr, _, s, _)| (*addr, s.score()))
                .collect::<Vec<_>>()
        );

        // 启动并发拨号任务
        let mut tasks: FuturesUnordered<JoinHandle<DialResult>> = FuturesUnordered::new();

        for (is_relay, addr, relay_url, _score, scheme) in &top {
            let scheme = scheme.clone();
            if *is_relay {
                let relay_connector = match &self.relay_connector {
                    Some(c) => Arc::clone(c),
                    None => continue,
                };
                // T-F2: is_relay=true 由构造方保证 relay_url 一定为 Some
                let Some(url) = relay_url.as_ref().cloned() else {
                    debug!("Relay candidate missing URL, skipping");
                    continue;
                };
                let local_device_id = self.local_device_id;
                let device_name = self.device_name.clone();
                let tls_config = Arc::clone(tls_config);
                let addr = *addr;

                let handle: JoinHandle<DialResult> = tokio::spawn(async move {
                    let start = Instant::now();
                    match relay_connector
                        .connect_via_relay(
                            &url,
                            device_id,
                            local_device_id,
                            &device_name,
                            &tls_config,
                        )
                        .await
                    {
                        Ok(conn) => {
                            let rtt = start.elapsed();
                            debug!("Relay dial via {} succeeded in {:?}", url, rtt);
                            Ok((conn, addr, rtt))
                        }
                        Err(e) => {
                            debug!("Relay dial via {} failed: {}", url, e);
                            Err(e)
                        }
                    }
                });

                tasks.push(handle);
            } else {
                let connector = self
                    .connectors
                    .get(&scheme)
                    .map(|e| Arc::clone(&*e))
                    .unwrap_or_else(|| Arc::clone(&*self.connector.read()));
                let local_device_id = self.local_device_id;
                let device_name = self.device_name.clone();
                let tls_config = Arc::clone(tls_config);
                let addr = *addr;

                let handle: JoinHandle<DialResult> = tokio::spawn(async move {
                    let start = Instant::now();
                    match connector
                        .connect(addr, device_id, local_device_id, &device_name, &tls_config)
                        .await
                    {
                        Ok(conn) => {
                            let rtt = start.elapsed();
                            debug!(
                                "Dial to {} via scheme {} succeeded in {:?}",
                                addr, scheme, rtt
                            );
                            Ok((conn, addr, rtt))
                        }
                        Err(e) => {
                            debug!("Dial to {} via scheme {} failed: {}", addr, scheme, e);
                            Err(e)
                        }
                    }
                });

                tasks.push(handle);
            }
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
                    crate::metrics::global().record_dial_rtt(
                        device_id.to_string(),
                        addr,
                        rtt,
                        top.iter()
                            .find(|(_, a, _, _, _)| *a == addr)
                            .map(|(_, _, _, _, scheme)| scheme.as_str())
                            .unwrap_or("unknown"),
                    );
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
        for (_, addr, _, _, _) in &top {
            self.record_failure(*addr);
        }

        Err(last_error
            .unwrap_or_else(|| SyncthingError::connection("all parallel dial attempts failed")))
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
    use std::net::IpAddr;

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
mod tests;
