//! TCP transport implementation.
//!
//! Implements the TCP listener and dialer
//! (reference: syncthing/lib/connections/tcp_listen.go, tcp_dial.go).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::{TcpListener as TokioTcpListener, TcpStream};
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use tracing::{debug, error, info, trace, warn};

use syncthing_core::{ConnectionType, Result, RetryConfig, SyncthingError};

use crate::connection::{BepConnection, TcpBiStream};
use crate::manager::ConnectionManagerHandle;
use crate::metrics;
use crate::tls::SyncthingTlsConfig;

/// Default TCP port for BEP connections.
pub const DEFAULT_TCP_PORT: u16 = syncthing_core::constants::DEFAULT_BEP_PORT;

/// Timeout for establishing a TCP connection.
pub const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Timeout for the TLS handshake.
pub const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Timeout for the BEP protocol handshake.
pub const PROTOCOL_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// TCP listener accepting incoming BEP connections.
pub struct SyncthingTcpListener {
    /// Configured bind address.
    // TODO: expose actual bound SocketAddr (currently only stores configured addr)
    #[allow(dead_code)]
    bind_addr: SocketAddr,
    /// Underlying TCP listener (present after `bind`).
    inner: Option<TokioTcpListener>,
    /// Connection manager handle.
    manager: ConnectionManagerHandle,
    /// Local device ID (used in the BEP Hello).
    local_device_id: syncthing_core::DeviceId,
    /// Device name advertised in the Hello.
    device_name: String,
    /// TLS configuration.
    tls_config: Arc<SyncthingTlsConfig>,
    /// Whether the accept loop is running.
    running: bool,
    /// Shutdown signal sender.
    shutdown_tx: Option<tokio::sync::mpsc::Sender<()>>,
}

impl SyncthingTcpListener {
    /// Creates a new TCP listener (not yet bound).
    pub fn new(
        bind_addr: SocketAddr,
        manager: ConnectionManagerHandle,
        local_device_id: syncthing_core::DeviceId,
        device_name: String,
        tls_config: Arc<SyncthingTlsConfig>,
    ) -> Self {
        Self {
            bind_addr,
            inner: None,
            manager,
            local_device_id,
            device_name,
            tls_config,
            running: false,
            shutdown_tx: None,
        }
    }

    /// Binds the listener and returns the actual local address.
    ///
    /// # Errors
    /// Returns an error if the bind fails or the actual local address cannot
    /// be queried.
    pub async fn bind(&mut self) -> Result<SocketAddr> {
        let listener = TokioTcpListener::bind(self.bind_addr).await.map_err(|e| {
            SyncthingError::connection(format!("failed to bind to {}: {}", self.bind_addr, e))
        })?;

        let actual_addr = listener
            .local_addr()
            .map_err(|e| SyncthingError::connection(format!("failed to get local addr: {}", e)))?;

        info!("TCP listener bound to {}", actual_addr);

        self.inner = Some(listener);
        self.bind_addr = actual_addr;

        Ok(actual_addr)
    }

    /// Runs the accept loop until `stop` is called.
    ///
    /// # Errors
    /// Returns an error if the listener was not bound first. Per-connection
    /// failures are logged and do not stop the loop.
    pub async fn run(mut self) -> Result<()> {
        let listener = self
            .inner
            .take()
            .ok_or_else(|| SyncthingError::connection("listener not bound"))?;

        self.running = true;
        info!("TCP listener started on {}", self.bind_addr);

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);

        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    info!("TCP listener received shutdown signal");
                    break;
                }
                result = listener.accept() => {
                    match result {
                        Ok((stream, peer_addr)) => {
                            debug!("Incoming TCP connection from {}", peer_addr);

                            if !self.manager.can_accept_connection() {
                                warn!(
                                    "Connection limit reached, rejecting incoming from {}",
                                    peer_addr
                                );
                                continue;
                            }

                            let manager = self.manager.clone();
                            let local_device_id = self.local_device_id;
                            let device_name = self.device_name.clone();
                            let tls_config = Arc::clone(&self.tls_config);

                            tokio::spawn(async move {
                                if let Err(e) = Self::handle_incoming(
                                    stream,
                                    peer_addr,
                                    manager,
                                    local_device_id,
                                    device_name,
                                    tls_config,
                                ).await {
                                    warn!("Failed to handle incoming connection from {}: {}", peer_addr, e);
                                }
                            });
                        }
                        Err(e) => {
                            error!("TCP accept error: {}", e);
                            // Brief pause to avoid CPU spinning on repeated accept errors.
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }
            }
        }

        info!("TCP listener stopped");
        Ok(())
    }

    /// Stops the listener.
    pub fn stop(&mut self) {
        self.running = false;
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.try_send(());
        }
        info!("TCP listener stopping...");
    }

    /// Handles an accepted incoming connection: TLS + BEP handshake, then
    /// registration with the connection manager.
    async fn handle_incoming(
        stream: TcpStream,
        _peer_addr: SocketAddr,
        manager: ConnectionManagerHandle,
        _local_device_id: syncthing_core::DeviceId,
        device_name: String,
        tls_config: Arc<SyncthingTlsConfig>,
    ) -> Result<()> {
        Self::configure_stream(&stream)?;

        let tls_start = std::time::Instant::now();
        let (tls_stream, device_id) = crate::tls::accept_tls_stream(stream, &tls_config).await?;
        metrics::global().record_tls_handshake(tls_start.elapsed());

        debug!(
            "Server TLS handshake completed: peer_device_id={}",
            device_id
        );

        let _handshake_guard = manager
            .begin_handshake(device_id, ConnectionType::Incoming)
            .map_err(|e| {
                warn!(
                    "Incoming handshake race lost for {} before BEP Hello: {}",
                    device_id, e
                );
                e
            })?;

        let mut tls_stream = tls_stream;
        let _remote_hello =
            crate::handshaker::BepHandshaker::server_handshake(&mut tls_stream, &device_name)
                .await?;

        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move { while event_rx.recv().await.is_some() {} });

        let conn = BepConnection::new(
            Box::new(TcpBiStream::server(tls_stream)),
            ConnectionType::Incoming,
            event_tx,
        )
        .await?;

        let conn_id = conn.id();

        conn.set_device_id(device_id);
        conn.set_state(syncthing_core::ConnectionState::ProtocolHandshakeComplete);
        manager.register_connection(device_id, conn).await?;

        info!("Incoming connection {} handled successfully", conn_id);

        Ok(())
    }

    /// Applies TCP options (TCP_NODELAY, keepalive) to a stream.
    fn configure_stream(stream: &TcpStream) -> Result<()> {
        stream
            .set_nodelay(true)
            .map_err(|e| SyncthingError::connection(format!("failed to set TCP_NODELAY: {}", e)))?;
        // TCP keepalive to prevent NAT/firewall silent drops (non-fatal)
        crate::transport::keepalive::apply_tcp_keepalive(
            stream,
            syncthing_core::TransportType::Tcp,
        );
        Ok(())
    }
}

/// Validates that an outbound address is safe to dial (SSRF protection).
///
/// Rejects the following address classes to prevent SSRF:
/// - Multicast (224.0.0.0/4, ff00::/8)
/// - Unspecified (0.0.0.0, ::)
/// - Link-local (169.254.0.0/16, fe80::/10)
///
/// NOTE: Loopback (127.0.0.0/8, ::1) is **not** rejected — multiple Syncthing
/// instances on the same host are a legitimate use case. Private addresses
/// (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, fc00::/7) are legitimate in
/// enterprise intranets.
///
/// Call this at the entry point of any **untrusted address source**
/// (discovery, relay, external configuration).
pub(crate) fn validate_outbound_addr(addr: &SocketAddr) -> Result<()> {
    let ip = addr.ip();
    if ip.is_multicast() {
        return Err(SyncthingError::config(format!(
            "SSRF prevention: refusing to dial multicast address {}",
            addr
        )));
    }
    if ip.is_unspecified() {
        return Err(SyncthingError::config(format!(
            "SSRF prevention: refusing to dial unspecified address {}",
            addr
        )));
    }
    // IpAddr::is_link_local is not yet stable; dispatch per address family instead.
    let is_link_local = match ip {
        std::net::IpAddr::V4(v4) => v4.is_link_local(),
        std::net::IpAddr::V6(v6) => {
            let segments = v6.segments();
            segments[0] & 0xffc0 == 0xfe80
        }
    };
    if is_link_local {
        return Err(SyncthingError::config(format!(
            "SSRF prevention: refusing to dial link-local address {}",
            addr
        )));
    }
    Ok(())
}

/// Establishes a single TCP connection and completes the TLS + BEP handshake.
///
/// # Errors
/// Returns an error on TCP connect timeout/failure, TLS handshake
/// timeout/failure, or BEP Hello exchange failure.
pub async fn connect_bep(
    addr: SocketAddr,
    target_device: syncthing_core::DeviceId,
    local_device_id: syncthing_core::DeviceId,
    device_name: &str,
    tls_config: &Arc<SyncthingTlsConfig>,
) -> Result<Arc<BepConnection>> {
    trace!("Dialing {} for device {}", addr, target_device);

    crate::manager::handshake::pre_handshake_yield(local_device_id, target_device).await;

    let stream = timeout(TCP_CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| SyncthingError::timeout("TCP connect timeout"))?
        .map_err(|e| SyncthingError::connection(format!("TCP connect failed: {}", e)))?;

    debug!("TCP connected to {}", addr);

    SyncthingTcpListener::configure_stream(&stream)?;

    let tls_start = std::time::Instant::now();
    let client_config = tls_config
        .client_config()
        .map_err(|e| SyncthingError::Tls(format!("failed to create client config: {}", e)))?;
    let connector = TlsConnector::from(Arc::new(client_config));
    let server_name = tokio_rustls::rustls::pki_types::ServerName::try_from("syncthing")
        .map_err(|_| SyncthingError::Tls("invalid server name".to_string()))?;
    let tls_stream = timeout(
        TLS_HANDSHAKE_TIMEOUT,
        connector.connect(server_name, stream),
    )
    .await
    .map_err(|_| SyncthingError::timeout("TLS handshake timeout"))?
    .map_err(|e| SyncthingError::Tls(format!("TLS handshake failed: {}", e)))?;
    metrics::global().record_tls_handshake(tls_start.elapsed());

    debug!("Client TLS handshake completed");

    let mut tls_stream = tls_stream;
    let _remote_hello =
        crate::handshaker::BepHandshaker::client_handshake(&mut tls_stream, device_name).await?;

    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(256);
    tokio::spawn(async move { while event_rx.recv().await.is_some() {} });

    let conn = BepConnection::new(
        Box::new(TcpBiStream::client(tls_stream)),
        ConnectionType::Outgoing,
        event_tx,
    )
    .await?;

    conn.set_device_id(target_device);

    conn.set_state(syncthing_core::ConnectionState::ProtocolHandshakeComplete);

    info!("Connection {} established to {}", conn.id(), addr);

    Ok(conn)
}

/// TCP dialer with retry support.
pub struct TcpDialer {
    /// Connection manager handle.
    manager: ConnectionManagerHandle,
    /// Local device ID.
    local_device_id: syncthing_core::DeviceId,
    /// Device name advertised in the Hello.
    device_name: String,
    /// Retry configuration.
    retry_config: RetryConfig,
    /// TLS configuration.
    tls_config: Arc<SyncthingTlsConfig>,
}

impl TcpDialer {
    /// Creates a new TCP dialer with default retry configuration.
    pub fn new(
        manager: ConnectionManagerHandle,
        local_device_id: syncthing_core::DeviceId,
        device_name: String,
        tls_config: Arc<SyncthingTlsConfig>,
    ) -> Self {
        Self {
            manager,
            local_device_id,
            device_name,
            retry_config: RetryConfig::default(),
            tls_config,
        }
    }

    /// Overrides the retry configuration.
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = config;
        self
    }

    /// Dials the given address with retries according to the retry configuration.
    ///
    /// # Errors
    /// Returns the last dial error after all attempts are exhausted.
    pub async fn dial(
        &self,
        addr: SocketAddr,
        target_device: syncthing_core::DeviceId,
    ) -> Result<Arc<BepConnection>> {
        let mut last_error = None;

        for attempt in 0..=self.retry_config.max_retries {
            match self.dial_once(addr, target_device).await {
                Ok(conn) => {
                    info!(
                        "Successfully connected to {} (device: {})",
                        addr, target_device
                    );
                    return Ok(conn);
                }
                Err(e) => {
                    warn!("Dial attempt {} to {} failed: {}", attempt + 1, addr, e);
                    last_error = Some(e);

                    if attempt < self.retry_config.max_retries {
                        let backoff = self.retry_config.backoff_duration(attempt);
                        debug!("Retrying in {:?}", backoff);
                        tokio::time::sleep(backoff).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| SyncthingError::connection("all dial attempts failed")))
    }

    /// Single dial attempt without retries.
    async fn dial_once(
        &self,
        addr: SocketAddr,
        target_device: syncthing_core::DeviceId,
    ) -> Result<Arc<BepConnection>> {
        connect_bep(
            addr,
            target_device,
            self.local_device_id,
            &self.device_name,
            &self.tls_config,
        )
        .await
    }

    /// Dials the given address and registers the connection with the manager.
    ///
    /// # Errors
    /// Returns an error if dialing fails after all retries, or if the manager
    /// rejects the registration.
    pub async fn dial_and_register(
        &self,
        addr: SocketAddr,
        target_device: syncthing_core::DeviceId,
    ) -> Result<()> {
        let conn = self.dial(addr, target_device).await?;

        self.manager
            .register_connection(target_device, conn)
            .await?;

        Ok(())
    }
}

/// TCP transport combining listener and dialer.
pub struct TcpTransport {
    /// Listener (taken when the accept loop starts).
    listener: Option<SyncthingTcpListener>,
    /// Dialer.
    dialer: TcpDialer,
    /// Bind address.
    // TODO: expose actual bound SocketAddr (currently only stores configured addr)
    #[allow(dead_code)]
    bind_addr: SocketAddr,
}

impl TcpTransport {
    /// Creates a new TCP transport (not yet started).
    pub fn new(
        bind_addr: SocketAddr,
        manager: ConnectionManagerHandle,
        local_device_id: syncthing_core::DeviceId,
        device_name: String,
        tls_config: Arc<SyncthingTlsConfig>,
    ) -> Self {
        let dialer = TcpDialer::new(
            manager.clone(),
            local_device_id,
            device_name.clone(),
            Arc::clone(&tls_config),
        );

        let listener =
            SyncthingTcpListener::new(bind_addr, manager, local_device_id, device_name, tls_config);

        Self {
            listener: Some(listener),
            dialer,
            bind_addr,
        }
    }

    /// Binds the listener and spawns the accept loop, returning the bound address.
    ///
    /// # Errors
    /// Returns an error if the transport was already started or the bind fails.
    pub async fn start(&mut self) -> Result<SocketAddr> {
        let addr = self
            .listener
            .as_mut()
            .ok_or_else(|| SyncthingError::config("TcpTransport::start called twice"))?
            .bind()
            .await?;

        let listener = self
            .listener
            .take()
            .ok_or_else(|| SyncthingError::config("TcpTransport::start listener missing"))?;
        tokio::spawn(async move {
            if let Err(e) = listener.run().await {
                error!("TCP listener error: {}", e);
            }
        });

        Ok(addr)
    }

    /// Returns a reference to the dialer.
    pub fn dialer(&self) -> &TcpDialer {
        &self.dialer
    }

    /// Dials the given device at the given address.
    ///
    /// # Errors
    /// Returns an error if dialing fails after all retries.
    pub async fn connect(
        &self,
        addr: SocketAddr,
        target_device: syncthing_core::DeviceId,
    ) -> Result<Arc<BepConnection>> {
        self.dialer.dial(addr, target_device).await
    }
}

/// Address parsing utilities.
pub mod addr {
    use super::*;

    /// Parses an address string into a `SocketAddr`.
    ///
    /// # Errors
    /// Returns an error if the string is not a valid socket address.
    pub fn parse_addr(addr_str: &str) -> Result<SocketAddr> {
        addr_str
            .parse()
            .map_err(|e| SyncthingError::config(format!("invalid address '{}': {}", addr_str, e)))
    }

    /// Parses multiple address strings.
    ///
    /// # Errors
    /// Returns an error if any entry is not a valid socket address.
    pub fn parse_addrs(addr_strs: &[&str]) -> Result<Vec<SocketAddr>> {
        addr_strs.iter().map(|s| parse_addr(s)).collect()
    }

    /// Builds the default listen address (all interfaces).
    pub fn default_listen_addr() -> SocketAddr {
        SocketAddr::from(([0, 0, 0, 0], DEFAULT_TCP_PORT))
    }

    /// Builds a localhost-only listen address.
    pub fn localhost_listen_addr() -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], DEFAULT_TCP_PORT))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syncthing_core::DeviceId;

    #[test]
    fn test_default_port() {
        assert_eq!(
            DEFAULT_TCP_PORT,
            syncthing_core::constants::DEFAULT_BEP_PORT
        );
    }

    #[test]
    fn test_addr_parse() {
        let addr = addr::parse_addr("127.0.0.1:22001").unwrap();
        assert_eq!(addr.port(), syncthing_core::constants::DEFAULT_BEP_PORT);
    }

    #[tokio::test]
    async fn test_tls_hello_exchange() {
        // Generate TLS configs for server and client
        let (server_cert, server_key) = crate::tls::generate_certificate("server").unwrap();
        let server_tls =
            Arc::new(crate::tls::SyncthingTlsConfig::from_pem(&server_cert, &server_key).unwrap());

        let (client_cert, client_key) = crate::tls::generate_certificate("client").unwrap();
        let client_tls =
            Arc::new(crate::tls::SyncthingTlsConfig::from_pem(&client_cert, &client_key).unwrap());

        // Start mock server
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_tls_clone = Arc::clone(&server_tls);

        let server_handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let server_config = server_tls_clone.server_config().unwrap();
            let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
            let mut tls_stream = acceptor.accept(stream).await.unwrap();

            // Server BEP handshake
            let hello = crate::handshaker::BepHandshaker::server_handshake(
                &mut tls_stream,
                "server-device",
            )
            .await
            .unwrap();

            hello
        });

        // Client connects using connect_bep
        let result = connect_bep(
            addr,
            DeviceId::default(),
            DeviceId::default(),
            "test-device",
            &client_tls,
        )
        .await;
        assert!(result.is_ok(), "connect_bep failed: {:?}", result.err());

        // Verify server received the correct protobuf Hello
        let received_hello = server_handle.await.unwrap();
        assert_eq!(received_hello.device_name, "test-device");
        assert_eq!(received_hello.client_name, "syncthing");
    }

    // ── validate_outbound_addr 测试 (v3.0.4 SSRF 防护) ──

    #[test]
    fn test_validate_outbound_addr_allows_normal() {
        assert!(validate_outbound_addr(&"192.168.1.1:22000".parse().unwrap()).is_ok());
        assert!(validate_outbound_addr(&"10.0.0.1:22000".parse().unwrap()).is_ok());
        assert!(validate_outbound_addr(&"8.8.8.8:22000".parse().unwrap()).is_ok());
        assert!(validate_outbound_addr(&"172.16.0.1:22000".parse().unwrap()).is_ok());
        // Loopback is allowed (legitimate for same-host syncthing instances)
        assert!(validate_outbound_addr(&"127.0.0.1:22000".parse().unwrap()).is_ok());
        assert!(validate_outbound_addr(&"[::1]:22000".parse().unwrap()).is_ok());
    }

    #[test]
    fn test_validate_outbound_addr_rejects_multicast() {
        assert!(validate_outbound_addr(&"224.0.0.1:22000".parse().unwrap()).is_err());
        assert!(validate_outbound_addr(&"239.255.255.255:22000".parse().unwrap()).is_err());
        assert!(validate_outbound_addr(&"[ff02::1]:22000".parse().unwrap()).is_err());
    }

    #[test]
    fn test_validate_outbound_addr_rejects_unspecified() {
        assert!(validate_outbound_addr(&"0.0.0.0:22000".parse().unwrap()).is_err());
        assert!(validate_outbound_addr(&"[::]:22000".parse().unwrap()).is_err());
    }

    #[test]
    fn test_validate_outbound_addr_rejects_link_local() {
        assert!(validate_outbound_addr(&"169.254.1.1:22000".parse().unwrap()).is_err());
        assert!(validate_outbound_addr(&"[fe80::1]:22000".parse().unwrap()).is_err());
    }
}
