//! REST API 服务器启动与配置存储

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use syncthing_core::traits::ConfigStore;
use syncthing_core::{DeviceId, Result, SyncthingError};
use tokio::net::TcpListener;
use tracing::{info, warn};

// API Key 认证中间件
// Middleware removed: API key checking will be done inside syncthing-api build_router
// after adding api_key field to ApiState.

/// 启动 REST API 服务器
pub async fn start_api_server(
    config_dir: &Path,
    sync_service: Arc<syncthing_sync::SyncService>,
    my_id: DeviceId,
    connection_handle: Option<syncthing_net::manager::ConnectionManagerHandle>,
) -> Result<tokio::task::JoinHandle<()>> {
    let config_path = config_dir.join("config.json");
    let config_store = Arc::new(syncthing_api::config::JsonConfigStore::new(&config_path));

    // 加载当前配置以获取 GUI 设置
    let config = config_store.load().await?;

    if !config.gui.enabled {
        info!("REST API is disabled (gui.enabled = false)");
        return Ok(tokio::spawn(async {}));
    }

    let addr: SocketAddr = config
        .gui
        .address
        .parse()
        .map_err(|e| SyncthingError::config(format!("invalid gui.address: {}", e)))?;

    let api_key = Arc::new(config.gui.api_key);

    let db = Some(sync_service.db());
    let mut state = syncthing_api::rest::ApiState::new(
        config_store,
        syncthing_api::events::EventBus::new(),
        Some(sync_service as Arc<dyn syncthing_core::traits::SyncModel>),
    );
    state.my_id = Some(my_id);
    state.api_key = Some(api_key.to_string());
    state.connection_manager = connection_handle;
    state.db = db;

    let router = syncthing_api::rest::RestApi::build_router(state);

    let listener = match TcpListener::bind(addr).await {
        Ok(l) => {
            info!("REST API server listening on {}", addr);
            l
        }
        Err(e) if addr.port() != 0 => {
            let fallback: SocketAddr = "0.0.0.0:0".parse().unwrap();
            warn!("Failed to bind REST API to {}, trying random port: {}", addr, e);
            let l = TcpListener::bind(fallback).await
                .map_err(|e2| SyncthingError::Network(format!("failed to bind REST API fallback: {}", e2)))?;
            info!("REST API server listening on fallback {}", l.local_addr().unwrap_or(fallback));
            l
        }
        Err(e) => return Err(SyncthingError::Network(format!("failed to bind REST API: {}", e))),
    };

    let handle = tokio::spawn(async move {
        let svc = router.into_make_service_with_connect_info::<SocketAddr>();
        if let Err(e) = axum::serve(listener, svc).await {
            warn!("REST API server error: {}", e);
        }
    });

    Ok(handle)
}
