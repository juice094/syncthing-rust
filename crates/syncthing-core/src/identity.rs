//! Device identity abstraction.
//!
//! The `Identity` trait decouples device identification from concrete
//! cryptography schemes (TLS, WireGuard, PSK, ...).
//!
//! See "docs/design/ADR-20261105-identity-abstraction.md".

use crate::DeviceId;
use std::fmt::Debug;

/// Abstraction over a device's identity, independent of the concrete
/// cryptography scheme.
///
/// Concrete implementations (e.g. `TlsIdentity`, `WireGuardIdentity`) are
/// provided by upper-layer crates. `Identity` identifies only; the handshake
/// belongs to the transport layer, so one device can hold several identities
/// at once and each transport picks the scheme that fits it best.
pub trait Identity: Send + Sync + Debug {
    /// Returns the device's unique identifier.
    ///
    /// For every implementation this is a fixed 32-byte identifier,
    /// independent of the underlying cryptography scheme.
    fn device_id(&self) -> DeviceId;

    /// Returns the identity scheme name.
    ///
    /// Example values: `"tls"`, `"wireguard"`, `"psk"`, `"noise"`.
    fn scheme(&self) -> &'static str;
}

/// Simplest identity implementation: holds only a DeviceId, with no
/// additional cryptographic material.
///
/// Used in tests, in-memory pipe scenarios, or as a transitional type for
/// callers not yet migrated to a concrete identity scheme.
#[derive(Debug, Clone)]
pub struct DeviceIdentity {
    id: DeviceId,
}

impl DeviceIdentity {
    /// Creates a device identity that holds only a device ID.
    pub fn new(id: DeviceId) -> Self {
        Self { id }
    }
}

impl Identity for DeviceIdentity {
    fn device_id(&self) -> DeviceId {
        self.id
    }

    fn scheme(&self) -> &'static str {
        "device"
    }
}

/// Provides convenient `device_id` access for `Arc<dyn Identity>`.
impl Identity for std::sync::Arc<dyn Identity> {
    fn device_id(&self) -> DeviceId {
        (**self).device_id()
    }

    fn scheme(&self) -> &'static str {
        (**self).scheme()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DeviceId;

    #[derive(Debug)]
    struct MockIdentity {
        id: DeviceId,
    }

    impl Identity for MockIdentity {
        fn device_id(&self) -> DeviceId {
            self.id
        }

        fn scheme(&self) -> &'static str {
            "mock"
        }
    }

    #[test]
    fn test_identity_trait() {
        let id = DeviceId::default();
        let identity = MockIdentity { id };
        assert_eq!(identity.device_id(), id);
        assert_eq!(identity.scheme(), "mock");
    }

    #[test]
    fn test_arc_identity() {
        let id = DeviceId::default();
        let identity: std::sync::Arc<dyn Identity> = std::sync::Arc::new(MockIdentity { id });
        assert_eq!(identity.device_id(), id);
        assert_eq!(identity.scheme(), "mock");
    }
}
