//! Standalone discovery tests for Agent-Net-3
//! 
//! This file contains tests for the DHT discovery implementation.
//! Run with: cargo test --test discovery_tests

use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use tokio::sync::RwLock;

/// Device ID (replicated from core for standalone test)
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct DeviceId([u8; 32]);

impl DeviceId {
    fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    
    fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
    
    fn short_id(&self) -> String {
        hex::encode(&self.0[..8])
    }
}

impl std::fmt::Debug for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DeviceId({})", self.short_id())
    }
}

/// Result type alias
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// DHT Discovery trait abstraction for testability
#[async_trait]
trait DhtDiscovery: Send + Sync {
    async fn publish(&self, device_id: &DeviceId, addresses: &[String]) -> Result<()>;
    async fn resolve(&self, device_id: &DeviceId) -> Result<Vec<String>>;
}

/// Mock DHT discovery for testing
struct MockDhtDiscovery {
    storage: Arc<RwLock<HashMap<DeviceId, Vec<String>>>>,
}

impl MockDhtDiscovery {
    fn new() -> Self {
        Self {
            storage: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl DhtDiscovery for MockDhtDiscovery {
    async fn publish(&self, device_id: &DeviceId, addresses: &[String]) -> Result<()> {
        let mut storage = self.storage.write().await;
        storage.insert(*device_id, addresses.to_vec());
        Ok(())
    }
    
    async fn resolve(&self, device_id: &DeviceId) -> Result<Vec<String>> {
        let storage = self.storage.read().await;
        Ok(storage.get(device_id).cloned().unwrap_or_default())
    }
}

/// Discovery trait
#[async_trait]
trait Discovery: Send + Sync {
    async fn lookup(&self, device: &DeviceId) -> Result<Vec<String>>;
    async fn announce(&self, device: &DeviceId, addresses: Vec<String>) -> Result<()>;
}

/// Iroh-based device discovery
struct IrohDiscovery {
    local_cache: Arc<RwLock<HashMap<DeviceId, Vec<String>>>>,
    dht_discovery: Option<Arc<dyn DhtDiscovery>>,
}

impl IrohDiscovery {
    fn new() -> Self {
        Self {
            local_cache: Arc::new(RwLock::new(HashMap::new())),
            dht_discovery: None,
        }
    }

    fn with_dht_discovery(discovery: Arc<dyn DhtDiscovery>) -> Self {
        Self {
            local_cache: Arc::new(RwLock::new(HashMap::new())),
            dht_discovery: Some(discovery),
        }
    }

    async fn add_device(&self, device: DeviceId, addresses: Vec<String>) {
        let mut cache = self.local_cache.write().await;
        cache.insert(device, addresses);
    }

    async fn is_cached(&self, device: &DeviceId) -> bool {
        let cache = self.local_cache.read().await;
        cache.contains_key(device)
    }

    async fn cache_stats(&self) -> (usize, Vec<DeviceId>) {
        let cache = self.local_cache.read().await;
        let devices: Vec<DeviceId> = cache.keys().copied().collect();
        (cache.len(), devices)
    }

    async fn clear_cache(&self) {
        let mut cache = self.local_cache.write().await;
        cache.clear();
    }

    async fn refresh_from_dht(&self, device: &DeviceId) -> Result<Vec<String>> {
        if let Some(ref dht) = self.dht_discovery {
            let addresses = dht.resolve(device).await?;
            if !addresses.is_empty() {
                let mut cache = self.local_cache.write().await;
                cache.insert(*device, addresses.clone());
            }
            Ok(addresses)
        } else {
            Ok(vec![])
        }
    }
}

#[async_trait]
impl Discovery for IrohDiscovery {
    async fn lookup(&self, device: &DeviceId) -> Result<Vec<String>> {
        // 1. Check local cache first
        {
            let cache = self.local_cache.read().await;
            if let Some(addresses) = cache.get(device) {
                return Ok(addresses.clone());
            }
        }

        // 2. If not in cache and we have DHT discovery, query DHT
        if self.dht_discovery.is_some() {
            match self.refresh_from_dht(device).await {
                Ok(addresses) => {
                    if !addresses.is_empty() {
                        return Ok(addresses);
                    }
                }
                Err(_) => {}
            }
        }

        // 3. Device not found
        Ok(vec![])
    }

    async fn announce(&self, device: &DeviceId, addresses: Vec<String>) -> Result<()> {
        // 1. Store in local cache
        {
            let mut cache = self.local_cache.write().await;
            cache.insert(*device, addresses.clone());
        }

        // 2. Publish to DHT if available
        if let Some(ref dht) = self.dht_discovery {
            dht.publish(device, &addresses).await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_device() -> DeviceId {
        let mut bytes = [0u8; 32];
        bytes[0] = 1;
        DeviceId::from_bytes(bytes)
    }

    fn test_device2() -> DeviceId {
        let mut bytes = [0u8; 32];
        bytes[0] = 2;
        DeviceId::from_bytes(bytes)
    }

    fn test_addresses() -> Vec<String> {
        vec!["192.168.1.1:22000".to_string()]
    }

    #[tokio::test]
    async fn test_discovery_new() {
        let discovery = IrohDiscovery::new();
        let device = test_device();
        let addresses = discovery.lookup(&device).await.unwrap();
        assert!(addresses.is_empty());
    }

    #[tokio::test]
    async fn test_discovery_announce() {
        let discovery = IrohDiscovery::new();
        let device = test_device();
        let addresses = test_addresses();

        discovery.announce(&device, addresses.clone()).await.unwrap();

        let found = discovery.lookup(&device).await.unwrap();
        assert_eq!(found, addresses);
    }

    #[tokio::test]
    async fn test_discovery_add_device() {
        let discovery = IrohDiscovery::new();
        let device = test_device();
        let addresses = vec!["10.0.0.1:22000".to_string()];

        discovery.add_device(device, addresses.clone()).await;

        let found = discovery.lookup(&device).await.unwrap();
        assert_eq!(found, addresses);
    }

    #[tokio::test]
    async fn test_discovery_multiple_devices() {
        let discovery = IrohDiscovery::new();
        let device1 = test_device();
        let device2 = test_device2();

        discovery.add_device(device1, vec!["192.168.1.1:22000".to_string()]).await;
        discovery.add_device(device2, vec!["192.168.1.2:22000".to_string()]).await;

        let found1 = discovery.lookup(&device1).await.unwrap();
        let found2 = discovery.lookup(&device2).await.unwrap();

        assert_eq!(found1[0], "192.168.1.1:22000");
        assert_eq!(found2[0], "192.168.1.2:22000");
    }

    #[tokio::test]
    async fn test_device_not_found() {
        let discovery = IrohDiscovery::new();
        let unknown_device = test_device();

        let result = discovery.lookup(&unknown_device).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    // ========== DHT Tests ==========

    #[tokio::test]
    async fn test_dht_announce() {
        let mock_dht = Arc::new(MockDhtDiscovery::new());
        let discovery = IrohDiscovery::with_dht_discovery(mock_dht.clone());
        let device = test_device();
        let addresses = test_addresses();

        discovery.announce(&device, addresses.clone()).await.unwrap();

        let cached = discovery.lookup(&device).await.unwrap();
        assert_eq!(cached, addresses);

        let dht_addresses = mock_dht.resolve(&device).await.unwrap();
        assert_eq!(dht_addresses, addresses);
    }

    #[tokio::test]
    async fn test_dht_lookup() {
        let mock_dht = Arc::new(MockDhtDiscovery::new());
        let discovery = IrohDiscovery::with_dht_discovery(mock_dht.clone());
        let device = test_device();
        let addresses = vec![
            "192.168.1.1:22000".to_string(),
            "10.0.0.1:22000".to_string(),
        ];

        mock_dht.publish(&device, &addresses).await.unwrap();

        let found = discovery.lookup(&device).await.unwrap();
        assert_eq!(found.len(), 2);
        assert!(found.contains(&"192.168.1.1:22000".to_string()));
        assert!(found.contains(&"10.0.0.1:22000".to_string()));

        assert!(discovery.is_cached(&device).await);
    }

    #[tokio::test]
    async fn test_dht_lookup_cache_priority() {
        let mock_dht = Arc::new(MockDhtDiscovery::new());
        let discovery = IrohDiscovery::with_dht_discovery(mock_dht.clone());
        let device = test_device();

        let cache_addresses = vec!["192.168.1.1:22000".to_string()];
        let dht_addresses = vec!["10.0.0.1:22000".to_string()];

        discovery.add_device(device, cache_addresses.clone()).await;
        mock_dht.publish(&device, &dht_addresses).await.unwrap();

        let found = discovery.lookup(&device).await.unwrap();
        assert_eq!(found, cache_addresses);
        assert_ne!(found, dht_addresses);
    }

    #[tokio::test]
    async fn test_cache_refresh() {
        let mock_dht = Arc::new(MockDhtDiscovery::new());
        let discovery = IrohDiscovery::with_dht_discovery(mock_dht.clone());
        let device = test_device();
        let addresses = vec!["192.168.1.100:22000".to_string()];

        assert!(!discovery.is_cached(&device).await);

        mock_dht.publish(&device, &addresses).await.unwrap();

        let found = discovery.lookup(&device).await.unwrap();
        assert_eq!(found, addresses);

        assert!(discovery.is_cached(&device).await);

        let (count, devices) = discovery.cache_stats().await;
        assert_eq!(count, 1);
        assert!(devices.contains(&device));
    }

    #[tokio::test]
    async fn test_cache_operations() {
        let discovery = IrohDiscovery::new();
        let device = test_device();
        let addresses = test_addresses();

        discovery.add_device(device, addresses.clone()).await;
        assert!(discovery.is_cached(&device).await);

        discovery.clear_cache().await;

        let (count, _) = discovery.cache_stats().await;
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_dht_fallback_on_not_found() {
        let mock_dht = Arc::new(MockDhtDiscovery::new());
        let discovery = IrohDiscovery::with_dht_discovery(mock_dht.clone());
        let device = test_device();

        let found = discovery.lookup(&device).await.unwrap();
        assert!(found.is_empty());
        assert!(!discovery.is_cached(&device).await);
    }

    #[tokio::test]
    async fn test_dht_announce_without_dht() {
        let discovery = IrohDiscovery::new();
        let device = test_device();
        let addresses = test_addresses();

        discovery.announce(&device, addresses.clone()).await.unwrap();

        let found = discovery.lookup(&device).await.unwrap();
        assert_eq!(found, addresses);
    }
}
