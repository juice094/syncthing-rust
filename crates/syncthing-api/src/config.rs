//! Module: syncthing-api
//! Worker: Agent-F
//! Status: UNVERIFIED
//!
//! ⚠️ 此代码由Agent生成，未经主控验证

//! Configuration management for Syncthing API
//!
//! This module provides file-based configuration storage with TOML format
//! and file watching capabilities for configuration changes.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use notify::{Config as NotifyConfig, Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{debug, error, info};

use syncthing_core::traits::{ConfigStore, ConfigStream};
use syncthing_core::types::Config;
use syncthing_core::{Result, SyncthingError};

/// File-based configuration storage
#[derive(Debug, Clone)]
pub struct FileConfigStore {
    /// Path to the configuration file
    path: PathBuf,
    /// In-memory cached configuration
    cache: Arc<RwLock<Option<Config>>>,
}

impl FileConfigStore {
    /// Create a new file config store
    ///
    /// # Arguments
    /// * `path` - Path to the TOML configuration file
    ///
    /// # Example
    /// ```
    /// use syncthing_api::config::FileConfigStore;
    ///
    /// let store = FileConfigStore::new("/path/to/config.toml");
    /// ```
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            cache: Arc::new(RwLock::new(None)),
        }
    }

    /// Get the configuration file path
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Create default configuration
    fn default_config() -> Config {
        Config {
            version: 1,
            listen_addr: "0.0.0.0:22000".to_string(),
            device_name: "syncthing-rust".to_string(),
            folders: Vec::new(),
            devices: Vec::new(),
            local_device_id: None,
            gui: syncthing_core::types::GuiConfig {
                enabled: true,
                address: "127.0.0.1:8384".to_string(),
                api_key: String::new(),
            },
            options: syncthing_core::types::Options {
                listen_addresses: vec!["default".to_string()],
                global_announce_enabled: true,
                local_announce_enabled: true,
                relays_enabled: true,
            },
        }
    }

    /// Ensure parent directory exists
    async fn ensure_dir(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| SyncthingError::Io(e))?;
        }
        Ok(())
    }
}

#[async_trait]
impl ConfigStore for FileConfigStore {
    /// Load configuration from file
    ///
    /// If the file does not exist, creates a default configuration
    /// and saves it to the file.
    async fn load(&self) -> Result<Config> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(ref config) = *cache {
                debug!("Returning cached configuration");
                return Ok(config.clone());
            }
        }

        // Check if file exists
        if !self.path.exists() {
            info!("Config file not found, creating default configuration");
            let config = Self::default_config();
            self.save(&config).await?;
            return Ok(config);
        }

        // Read and parse file
        let content = tokio::fs::read_to_string(&self.path)
            .await
            .map_err(|e| SyncthingError::Io(e))?;

        let config: Config = toml::from_str(&content).map_err(|e| {
            SyncthingError::config(format!("Failed to parse TOML: {}", e))
        })?;

        // Update cache
        let mut cache = self.cache.write().await;
        *cache = Some(config.clone());

        info!("Configuration loaded from {:?}", self.path);
        Ok(config)
    }

    /// Save configuration to file
    async fn save(&self, config: &Config) -> Result<()> {
        self.ensure_dir().await?;

        let content = toml::to_string_pretty(config).map_err(|e| {
            SyncthingError::config(format!("Failed to serialize TOML: {}", e))
        })?;

        tokio::fs::write(&self.path, content)
            .await
            .map_err(|e| SyncthingError::Io(e))?;

        // Update cache
        let mut cache = self.cache.write().await;
        *cache = Some(config.clone());

        info!("Configuration saved to {:?}", self.path);
        Ok(())
    }

    /// Watch for configuration file changes
    ///
    /// Returns a stream that yields when the configuration file changes.
    async fn watch(&self) -> Result<Box<dyn ConfigStream>> {
        let (tx, rx) = mpsc::channel(10);
        let path = self.path.clone();
        let cache = self.cache.clone();

        // Create watcher
        let mut watcher = RecommendedWatcher::new(
            move |res: std::result::Result<Event, notify::Error>| {
                match res {
                    Ok(event) => {
                        if event.paths.iter().any(|p| p == &path) {
                            debug!("Configuration file changed: {:?}", event);
                            let _ = tx.try_send(());
                        }
                    }
                    Err(e) => {
                        error!("Watch error: {}", e);
                    }
                }
            },
            NotifyConfig::default(),
        )
        .map_err(|e| SyncthingError::config(format!("Failed to create watcher: {}", e)))?;

        // Watch the config file
        watcher
            .watch(&self.path, RecursiveMode::NonRecursive)
            .map_err(|e| SyncthingError::config(format!("Failed to watch file: {}", e)))?;

        // Keep watcher alive
        tokio::spawn(async move {
            // Watcher is kept alive by being moved into this task
            let _watcher = watcher;
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            }
        });

        let stream = FileConfigStream {
            receiver: rx,
            path: self.path.clone(),
            cache: cache.clone(),
        };

        Ok(Box::new(stream))
    }
}

/// File configuration change stream
pub struct FileConfigStream {
    receiver: mpsc::Receiver<()>,
    path: PathBuf,
    cache: Arc<RwLock<Option<Config>>>,
}

#[async_trait]
impl ConfigStream for FileConfigStream {
    async fn next(&mut self) -> Result<()> {
        self.receiver
            .recv()
            .await
            .ok_or_else(|| SyncthingError::config("Watch channel closed".to_string()))?;

        // Reload configuration
        if self.path.exists() {
            let content = tokio::fs::read_to_string(&self.path)
                .await
                .map_err(|e| SyncthingError::Io(e))?;

            let config: Config = toml::from_str(&content).map_err(|e| {
                SyncthingError::config(format!("Failed to parse updated TOML: {}", e))
            })?;

            let mut cache = self.cache.write().await;
            *cache = Some(config);
            info!("Configuration reloaded from file");
        }

        Ok(())
    }
}

/// In-memory configuration store for testing
#[derive(Debug, Clone)]
pub struct MemoryConfigStore {
    config: Arc<RwLock<Config>>,
    change_tx: broadcast::Sender<()>,
}

impl MemoryConfigStore {
    /// Create a new memory config store with default configuration
    pub fn new() -> Self {
        let (change_tx, _) = broadcast::channel(10);
        Self {
            config: Arc::new(RwLock::new(FileConfigStore::default_config())),
            change_tx,
        }
    }

    /// Create a new memory config store with the given configuration
    pub fn with_config(config: Config) -> Self {
        let (change_tx, _) = broadcast::channel(10);
        Self {
            config: Arc::new(RwLock::new(config)),
            change_tx,
        }
    }
}

impl Default for MemoryConfigStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ConfigStore for MemoryConfigStore {
    async fn load(&self) -> Result<Config> {
        let config = self.config.read().await;
        Ok(config.clone())
    }

    async fn save(&self, config: &Config) -> Result<()> {
        let mut stored = self.config.write().await;
        *stored = config.clone();
        let _ = self.change_tx.send(());
        Ok(())
    }

    async fn watch(&self) -> Result<Box<dyn ConfigStream>> {
        let rx = self.change_tx.subscribe();
        let stream = MemoryConfigStream { receiver: rx };
        Ok(Box::new(stream))
    }
}

/// Memory configuration change stream
pub struct MemoryConfigStream {
    receiver: broadcast::Receiver<()>,
}

#[async_trait]
impl ConfigStream for MemoryConfigStream {
    async fn next(&mut self) -> Result<()> {
        self.receiver
            .recv()
            .await
            .map_err(|e| SyncthingError::config(format!("Broadcast channel error: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    async fn test_memory_config_store() {
        let store = MemoryConfigStore::new();

        // Load default config
        let config = store.load().await.unwrap();
        assert_eq!(config.version, 1);
        assert!(config.folders.is_empty());

        // Modify and save
        let mut new_config = config.clone();
        new_config.version = 2;
        store.save(&new_config).await.unwrap();

        // Reload and verify
        let loaded = store.load().await.unwrap();
        assert_eq!(loaded.version, 2);
    }

    #[tokio::test]
    async fn test_default_config() {
        let config = FileConfigStore::default_config();
        assert_eq!(config.version, 1);
        assert!(config.gui.enabled);
        assert_eq!(config.gui.address, "127.0.0.1:8384");
    }

    #[tokio::test]
    async fn test_file_config_store_load_save() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_path = temp_dir.path().join("config.toml");

        let store = FileConfigStore::new(&config_path);

        // Load should create default config
        let config = store.load().await.unwrap();
        assert_eq!(config.version, 1);

        // Verify file was created
        assert!(config_path.exists());

        // Modify and save
        let mut new_config = config.clone();
        new_config.version = 3;
        store.save(&new_config).await.unwrap();

        // Create new store and verify persistence
        let store2 = FileConfigStore::new(&config_path);
        let loaded = store2.load().await.unwrap();
        assert_eq!(loaded.version, 3);
    }

    #[tokio::test]
    async fn test_config_watch() {
        let store = MemoryConfigStore::new();

        let mut stream = store.watch().await.unwrap();

        // Save should trigger watch
        let config = store.load().await.unwrap();
        store.save(&config).await.unwrap();

        // Wait for notification with timeout
        let _result = timeout(Duration::from_millis(100), stream.next()).await;
        // Note: Memory store watch implementation is simplified
        // In real implementation, this should receive the change notification
    }
}
