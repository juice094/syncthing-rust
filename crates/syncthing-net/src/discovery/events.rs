//! Discovery events
//!
//! Events emitted by the discovery subsystem when devices are found
//! or their addresses change.

use syncthing_core::DeviceId;

/// Event emitted when a device is discovered or its addresses change.
#[derive(Debug, Clone)]
pub enum DiscoveryEvent {
    /// A new device was discovered.
    DeviceDiscovered {
        device_id: DeviceId,
        addresses: Vec<String>,
        source: DiscoverySource,
    },
    /// Device addresses were updated.
    AddressesUpdated {
        device_id: DeviceId,
        added: Vec<String>,
        removed: Vec<String>,
    },
}

/// Source of discovery information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoverySource {
    /// Local broadcast / multicast.
    Local,
    /// Global discovery server.
    Global,
    /// Manually configured.
    Config,
    /// Relay pool.
    Relay,
}
