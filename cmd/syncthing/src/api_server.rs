//! REST API 服务器启动与配置存储

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use syncthing_core::traits::{ConfigStore, FolderDatabase};
use syncthing_core::{DeviceId, Result, SyncthingError};
use tokio::net::TcpListener;
use tracing::{info, warn};

#[derive(Clone)]
struct DbAdapter(Arc<dyn syncthing_sync::database::LocalDatabase>);

#[async_trait]
impl FolderDatabase for DbAdapter {
    async fn get_folder_files(&self, folder_id: &str) -> syncthing_core::Result<Vec<syncthing_core::types::FileInfo>> {
        self.0.get_folder_files(folder_id).await.map_err(|e| syncthing_core::SyncthingError::Storage(e.to_string()))
    }
}

// API Key 认证中间件
// Middleware removed: API key checking will be done inside syncthing-api build_router
// after adding api_key field to ApiState.

/// 启动 REST API 服务器
pub async fn start_api_server(
    config_dir: &Path,
    sync_service: Arc<syncthing_sync::SyncService>,
    my_id: DeviceId,
    connection_handle: Option<syncthing_net::manager::ConnectionManagerHandle>,
) -> Result<(tokio::task::JoinHandle<()>, SocketAddr)> {
    let config_path = config_dir.join("config.json");
    let config_store = Arc::new(syncthing_api::config::JsonConfigStore::new(&config_path));

    // 加载当前配置以获取 GUI 设置
    let config = config_store.load().await?;

    if !config.gui.enabled {
        info!("REST API is disabled (gui.enabled = false)");
        return Ok((tokio::spawn(async {}), SocketAddr::from(([0, 0, 0, 0], 0))));
    }

    let addr: SocketAddr = config
        .gui
        .address
        .parse()
        .map_err(|e| SyncthingError::config(format!("invalid gui.address: {}", e)))?;

    let api_key = Arc::new(config.gui.api_key);

    let mut state = syncthing_api::rest::ApiState::new(
        config_store,
        syncthing_api::events::EventBus::new(),
        Some(sync_service.clone() as Arc<dyn syncthing_core::traits::SyncModel>),
    );
    state.my_id = Some(my_id);
    state.api_key = Some(api_key.to_string());
    state.connection_manager = connection_handle.map(|h| Arc::new(h) as Arc<dyn syncthing_core::traits::ConnectionManager>);
    state.db = Some(Arc::new(DbAdapter(sync_service.db())) as Arc<dyn FolderDatabase>);

    let router = syncthing_api::rest::RestApi::build_router(state);

    let listener = match TcpListener::bind(addr).await {
        Ok(l) => {
            info!("REST API server listening on {}", addr);
            l
        }
        Err(e) if addr.port() != 0 => {
            let fallback: SocketAddr = SocketAddr::from(([0, 0, 0, 0], 0));
            warn!("Failed to bind REST API to {}, trying random port: {}", addr, e);
            let l = TcpListener::bind(fallback).await
                .map_err(|e2| SyncthingError::Network(format!("failed to bind REST API fallback: {}", e2)))?;
            info!("REST API server listening on fallback {}", l.local_addr().unwrap_or(fallback));
            l
        }
        Err(e) => return Err(SyncthingError::Network(format!("failed to bind REST API: {}", e))),
    };

    let addr = listener.local_addr()
        .map_err(|e| SyncthingError::Network(format!("failed to get local addr: {}", e)))?;

    let handle = tokio::spawn(async move {
        let svc = router.into_make_service_with_connect_info::<SocketAddr>();
        if let Err(e) = axum::serve(listener, svc).await {
            warn!("REST API server error: {}", e);
        }
    });

    Ok((handle, addr))
}
