//! Connection-related types.
//!
//! Connection state, address types, statistics, priority, direction, and
//! retry configuration.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ConnectionState {
    /// Initial state.
    #[default]
    Initial,
    /// Connecting.
    Connecting,
    /// Connected.
    Connected,
    /// TLS handshake complete.
    TlsHandshakeComplete,
    /// Protocol handshake complete (Hello exchanged).
    ProtocolHandshakeComplete,
    /// ClusterConfig exchange complete.
    ClusterConfigComplete,
    /// Disconnecting.
    Disconnecting,
    /// Disconnected.
    Disconnected,
    /// Error state.
    Error,
}

impl ConnectionState {
    /// Returns whether the connection is active (can carry data).
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            ConnectionState::Connected
                | ConnectionState::TlsHandshakeComplete
                | ConnectionState::ProtocolHandshakeComplete
                | ConnectionState::ClusterConfigComplete
        )
    }

    /// Returns whether messages can be sent on this connection.
    pub fn can_send(&self) -> bool {
        matches!(
            self,
            ConnectionState::ProtocolHandshakeComplete | ConnectionState::ClusterConfigComplete
        )
    }

    /// Returns whether the connection has terminated.
    pub fn is_terminated(&self) -> bool {
        matches!(self, ConnectionState::Disconnected | ConnectionState::Error)
    }
}

/// Address type for a connection path.
///
/// Serializes as a human-readable URL string (`tcp://host:port`,
/// `quic://host:port`, `relay://url`, or `dynamic`). Deserialization also
/// accepts the legacy tagged-enum form (`{"Tcp": "..."}` / `"Dynamic"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AddressType {
    /// TCP address.
    Tcp(String),
    /// QUIC address.
    Quic(String),
    /// Relay address.
    Relay(String),
    /// Discovered dynamically.
    Dynamic,
}

impl AddressType {
    /// Returns the address string without the scheme.
    pub fn as_str(&self) -> &str {
        match self {
            AddressType::Tcp(addr) => addr,
            AddressType::Quic(addr) => addr,
            AddressType::Relay(addr) => addr,
            AddressType::Dynamic => "dynamic",
        }
    }
}

impl fmt::Display for AddressType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AddressType::Tcp(addr) => write!(f, "tcp://{}", addr),
            AddressType::Quic(addr) => write!(f, "quic://{}", addr),
            AddressType::Relay(url) => write!(f, "relay://{}", url),
            AddressType::Dynamic => write!(f, "dynamic"),
        }
    }
}

impl Serialize for AddressType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for AddressType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct AddressTypeVisitor;

        impl<'de> serde::de::Visitor<'de> for AddressTypeVisitor {
            type Value = AddressType;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(
                    formatter,
                    "a URL string (e.g. \"tcp://host:port\") or an object like {{\"Tcp\": \"host:port\"}}"
                )
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                match value {
                    "Dynamic" | "dynamic" => Ok(AddressType::Dynamic),
                    _ => parse_address_str(value).map_err(serde::de::Error::custom),
                }
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: serde::de::MapAccess<'de>,
            {
                let key: String = map
                    .next_key()?
                    .ok_or_else(|| serde::de::Error::custom("empty address object"))?;
                match key.as_str() {
                    "Tcp" => {
                        let value: String = map.next_value()?;
                        Ok(AddressType::Tcp(value))
                    }
                    "Quic" => {
                        let value: String = map.next_value()?;
                        Ok(AddressType::Quic(value))
                    }
                    "Relay" => {
                        let value: String = map.next_value()?;
                        Ok(AddressType::Relay(value))
                    }
                    "Dynamic" => {
                        let (): () = map.next_value()?;
                        Ok(AddressType::Dynamic)
                    }
                    _ => Err(serde::de::Error::custom(format!(
                        "unknown address type variant: {}",
                        key
                    ))),
                }
            }
        }

        deserializer.deserialize_any(AddressTypeVisitor)
    }
}

fn parse_address_str(s: &str) -> Result<AddressType, String> {
    if let Some(addr) = s.strip_prefix("tcp://") {
        Ok(AddressType::Tcp(addr.to_string()))
    } else if let Some(addr) = s.strip_prefix("quic://") {
        Ok(AddressType::Quic(addr.to_string()))
    } else if let Some(url) = s.strip_prefix("relay://") {
        Ok(AddressType::Relay(url.to_string()))
    } else {
        Err(format!(
            "Invalid address format '{}'. Expected one of: tcp://host:port, quic://host:port, relay://url, dynamic",
            s
        ))
    }
}

/// Connection statistics.
#[derive(Debug, Clone, Default)]
pub struct ConnectionStats {
    /// Time the connection was established.
    pub connected_at: Option<DateTime<Utc>>,
    /// Time of last activity.
    pub last_activity: Option<DateTime<Utc>>,
    /// Bytes sent.
    pub bytes_sent: u64,
    /// Bytes received.
    pub bytes_received: u64,
    /// Messages sent.
    pub messages_sent: u64,
    /// Messages received.
    pub messages_received: u64,
    /// Number of retries.
    pub retry_count: u32,
}

/// Connection priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum ConnectionPriority {
    /// Lowest priority.
    Lowest = 0,
    /// Low priority.
    Low = 1,
    /// Normal priority.
    #[default]
    Normal = 2,
    /// High priority.
    High = 3,
    /// Highest priority.
    Highest = 4,
}

/// Connection direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionType {
    /// Incoming connection.
    Incoming,
    /// Outgoing (dialed) connection.
    Outgoing,
}

/// Retry policy configuration.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retries.
    pub max_retries: u32,
    /// Initial backoff (milliseconds).
    pub initial_backoff_ms: u64,
    /// Maximum backoff (milliseconds).
    pub max_backoff_ms: u64,
    /// Backoff multiplier.
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 10,
            initial_backoff_ms: 1000,
            max_backoff_ms: 300000,
            backoff_multiplier: 2.0,
        }
    }
}

impl RetryConfig {
    /// Computes the backoff duration for the given attempt, adding ±25% jitter.
    pub fn backoff_duration(&self, attempt: u32) -> std::time::Duration {
        if attempt == 0 {
            return std::time::Duration::from_millis(self.initial_backoff_ms);
        }

        let multiplier = self.backoff_multiplier.powi(attempt as i32);
        let backoff_ms = (self.initial_backoff_ms as f64 * multiplier) as u64;
        let backoff_ms = backoff_ms.min(self.max_backoff_ms);

        let jitter = rand::random::<f64>() * 0.5 - 0.25;
        let jittered_ms = (backoff_ms as f64 * (1.0 + jitter)) as u64;

        std::time::Duration::from_millis(jittered_ms.max(100))
    }
}
