//! Parallel dialer.
//!
//! Implements concurrent dialing over multiple addresses, per-address quality
//! scoring, and best-connection selection.

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

/// Address type preference (affects scoring order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AddressTypePreference {
    /// Relay address (lowest priority).
    Relay,
    /// Public (WAN) address.
    Wan,
    /// LAN address (highest priority).
    Lan,
}

/// Per-address quality score record.
#[derive(Debug, Clone)]
pub struct AddressScore {
    /// Target address.
    pub address: SocketAddr,
    /// Most recent handshake RTT.
    pub rtt: Option<Duration>,
    /// Success count.
    pub success_count: u32,
    /// Failure count.
    pub failure_count: u32,
    /// Time of last success.
    pub last_success: Option<Instant>,
    /// Address type preference.
    pub address_type: AddressTypePreference,
}

impl AddressScore {
    /// Computes the current score for this address.
    ///
    /// Scoring rules:
    /// - Base: LAN > WAN > Relay.
    /// - Lower RTT scores higher (capped at 400 ms).
    /// - +10_000 per success; extra credit for successes within the last hour.
    /// - -50_000 per failure.
    pub fn score(&self) -> u64 {
        let mut score: u64 = 0;

        match self.address_type {
            AddressTypePreference::Lan => score += 1_000_000,
            AddressTypePreference::Wan => score += 500_000,
            AddressTypePreference::Relay => score += 100_000,
        }

        if let Some(rtt) = self.rtt {
            let rtt_ms = rtt.as_millis() as u64;
            let rtt_bonus = 400_u64.saturating_sub(rtt_ms);
            score += rtt_bonus * 100;
        }

        score += self.success_count as u64 * 10_000;

        if let Some(last) = self.last_success {
            let elapsed_secs = last.elapsed().as_secs();
            if elapsed_secs < 3600 {
                score += (3600 - elapsed_secs) * 10;
            }
        }

        score = score.saturating_sub(self.failure_count as u64 * 50_000);

        score
    }
}

/// Dial connector abstraction (allows test doubles).
#[async_trait::async_trait]
pub trait DialConnector: Send + Sync {
    /// Performs the TCP + BEP handshake against a single address.
    async fn connect(
        &self,
        addr: SocketAddr,
        device_id: DeviceId,
        local_device_id: DeviceId,
        device_name: &str,
        tls_config: &Arc<SyncthingTlsConfig>,
    ) -> Result<Arc<BepConnection>, SyncthingError>;
}

/// Connector backed by the real TCP transport.
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

/// Relay dial connector abstraction.
#[async_trait::async_trait]
pub trait RelayDialConnector: Send + Sync {
    /// Establishes a BEP connection through a relay server.
    async fn connect_via_relay(
        &self,
        relay_url: &str,
        target_device_id: DeviceId,
        local_device_id: DeviceId,
        device_name: &str,
        tls_config: &Arc<SyncthingTlsConfig>,
    ) -> Result<Arc<BepConnection>, SyncthingError>;
}

/// Connector backed by the real Relay protocol.
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
        crate::manager::handshake::pre_handshake_yield(local_device_id, target_device_id).await;

        crate::relay::connect_bep_via_relay(relay_url, target_device_id, device_name, tls_config)
            .await
    }
}

/// Parallel dialer.
///
/// Maintains per-address history scores and concurrently dials multiple
/// candidate addresses, returning the first successful connection.
pub struct ParallelDialer {
    /// Per-address score table.
    scores: DashMap<SocketAddr, AddressScore>,
    /// Per-device address scores (redundant cache for fast manager lookups).
    device_scores: DashMap<DeviceId, Vec<AddressScore>>,
    /// Local device ID.
    local_device_id: DeviceId,
    /// Device name (used in the Hello).
    device_name: String,
    /// Default connector (backward compatible).
    connector: RwLock<Arc<dyn DialConnector>>,
    /// Scheme → connector map (multi-transport routing).
    connectors: DashMap<String, Arc<dyn DialConnector>>,
    /// Optional relay connector.
    relay_connector: Option<Arc<dyn RelayDialConnector>>,
}

impl ParallelDialer {
    /// Creates a dialer with a custom default connector.
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

    /// Creates a dialer with the default TCP connector and a relay connector.
    pub fn with_tcp_connector(local_device_id: DeviceId, device_name: String) -> Self {
        let mut dialer = Self::new(local_device_id, device_name, Arc::new(TcpBepConnector));
        dialer.relay_connector = Some(Arc::new(RelayBepConnector));
        dialer
    }

    /// Sets the relay connector.
    pub fn set_relay_connector(&mut self, connector: Arc<dyn RelayDialConnector>) {
        self.relay_connector = Some(connector);
    }

    /// Replaces the default connector (used when transports register).
    pub fn set_connector(&self, connector: Arc<dyn DialConnector>) {
        *self.connector.write() = connector;
    }

    /// Registers a scheme-specific connector.
    pub fn register_connector(&self, scheme: impl Into<String>, connector: Arc<dyn DialConnector>) {
        let scheme = scheme.into();
        debug!("Registering dial connector for scheme: {}", scheme);
        self.connectors.insert(scheme, connector);
    }

    /// Returns the list of registered schemes.
    pub fn registered_schemes(&self) -> Vec<String> {
        self.connectors.iter().map(|e| e.key().clone()).collect()
    }

    /// Returns the score record for an address, initializing it if absent.
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

    /// Records a successful dial to an address.
    pub fn record_success(&self, addr: SocketAddr, rtt: Duration) {
        if let Some(mut score) = self.scores.get_mut(&addr) {
            score.rtt = Some(rtt);
            score.success_count += 1;
            score.last_success = Some(Instant::now());
        }
    }

    /// Records a failed dial to an address.
    pub fn record_failure(&self, addr: SocketAddr) {
        if let Some(mut score) = self.scores.get_mut(&addr) {
            score.failure_count += 1;
        }
    }

    /// Concurrent dial (backward-compatible wrapper).
    ///
    /// Calls `dial_with_schemes` with `None` as the scheme list, so all direct
    /// addresses use the default connector.
    ///
    /// # Errors
    /// Returns an error if no candidates exist or all dial attempts fail.
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

    /// Concurrent dial with scheme-aware routing.
    ///
    /// 1. Sorts direct and relay candidates by historical score.
    /// 2. Dials the top 3 candidates concurrently.
    /// 3. The first successful handshake wins; remaining tasks are aborted.
    /// 4. Score statistics are updated for the winner and the losers.
    ///
    /// # Errors
    /// Returns an error if no candidates exist or all dial attempts fail.
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

        let mut candidates: Vec<(bool, SocketAddr, Option<String>, AddressScore, String)> =
            Vec::new();

        for (i, addr) in addresses.iter().enumerate() {
            let scheme = address_schemes
                .as_ref()
                .and_then(|v| v.get(i).cloned())
                .unwrap_or_else(|| "tcp".to_string());
            candidates.push((false, *addr, None, self.get_or_create_score(*addr), scheme));
        }

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

        candidates.sort_by_key(|(_, _, _, s, _)| Reverse(s.score()));

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

        let mut tasks: FuturesUnordered<JoinHandle<DialResult>> = FuturesUnordered::new();

        for (is_relay, addr, relay_url, _score, scheme) in &top {
            let scheme = scheme.clone();
            if *is_relay {
                let relay_connector = match &self.relay_connector {
                    Some(c) => Arc::clone(c),
                    None => continue,
                };
                // INVARIANT (T-F2): is_relay=true candidates always carry a relay URL.
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

        for (_, addr, _, _, _) in &top {
            self.record_failure(*addr);
        }

        Err(last_error
            .unwrap_or_else(|| SyncthingError::connection("all parallel dial attempts failed")))
    }

    /// Returns a reference to the internal score table (for manager queries).
    pub fn address_scores(&self) -> &DashMap<SocketAddr, AddressScore> {
        &self.scores
    }

    /// Returns all address scores cached for a device.
    pub fn device_address_scores(&self, device_id: &DeviceId) -> Option<Vec<AddressScore>> {
        self.device_scores.get(device_id).map(|e| e.clone())
    }

    /// Replaces the cached address score list for a device.
    pub fn update_device_scores(&self, device_id: DeviceId, scores: Vec<AddressScore>) {
        self.device_scores.insert(device_id, scores);
    }
}

/// Infers the address type preference from IP characteristics.
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
