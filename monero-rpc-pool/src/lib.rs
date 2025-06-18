use std::sync::Arc;

use anyhow::Result;
use axum::{
    routing::{any, get},
    Router,
};
use monero::Network;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tower_http::cors::CorsLayer;
use tracing::{error, info};

fn network_to_string(network: &Network) -> String {
    match network {
        Network::Mainnet => "mainnet".to_string(),
        Network::Stagenet => "stagenet".to_string(),
        Network::Testnet => "testnet".to_string(),
    }
}

pub mod config;
pub mod database;
pub mod discovery;
pub mod pool;
pub mod simple_handlers;

use config::Config;
use database::Database;
use discovery::NodeDiscovery;
use pool::{NodePool, PoolStatus};
use simple_handlers::{simple_proxy_handler, simple_stats_handler};

#[derive(Clone)]
pub struct AppState {
    pub node_pool: Arc<RwLock<NodePool>>,
}

/// Manages background tasks for the RPC pool
pub struct TaskManager {
    pub status_update_handle: JoinHandle<()>,
    pub discovery_handle: JoinHandle<()>,
}

impl Drop for TaskManager {
    fn drop(&mut self) {
        self.status_update_handle.abort();
        self.discovery_handle.abort();
    }
}

/// Information about a running RPC pool server
#[derive(Debug, Clone)]
pub struct ServerInfo {
    pub port: u16,
    pub host: String,
}

async fn create_app_with_receiver(
    config: Config,
    network: Network,
) -> Result<(
    Router,
    tokio::sync::broadcast::Receiver<PoolStatus>,
    TaskManager,
)> {
    // Initialize database
    let db = Database::new_with_data_dir(config.data_dir.clone()).await?;

    // Initialize node pool with network
    let network_str = network_to_string(&network);
    let (node_pool, status_receiver) = NodePool::new(db.clone(), network_str.clone());
    let node_pool = Arc::new(RwLock::new(node_pool));

    // Initialize discovery service
    let discovery = NodeDiscovery::new(db.clone());

    // Start background tasks
    let node_pool_for_health_check = node_pool.clone();
    let status_update_handle = tokio::spawn(async move {
        loop {
            // Publish status update after health check
            let pool_guard = node_pool_for_health_check.read().await;
            if let Err(e) = pool_guard.publish_status_update().await {
                error!("Failed to publish status update after health check: {}", e);
            }

            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        }
    });

    // Start periodic discovery task
    let discovery_clone = discovery.clone();
    let network_clone = network;
    let discovery_handle = tokio::spawn(async move {
        if let Err(e) = discovery_clone.periodic_discovery_task(network_clone).await {
            error!(
                "Periodic discovery task failed for network {}: {}",
                network_to_string(&network_clone),
                e
            );
        }
    });

    let task_manager = TaskManager {
        status_update_handle,
        discovery_handle,
    };

    let app_state = AppState { node_pool };

    // Build the app
    let app = Router::new()
        .route("/stats", get(simple_stats_handler))
        .route("/*path", any(simple_proxy_handler))
        .layer(CorsLayer::permissive())
        .with_state(app_state);

    Ok((app, status_receiver, task_manager))
}

pub async fn create_app(config: Config, network: Network) -> Result<Router> {
    let (app, _, _task_manager) = create_app_with_receiver(config, network).await?;
    // Note: task_manager is dropped here, so tasks will be aborted when this function returns
    // This is intentional for the simple create_app use case
    Ok(app)
}

/// Create an app with a custom data directory for the database
pub async fn create_app_with_data_dir(
    config: Config,
    network: Network,
    data_dir: std::path::PathBuf,
) -> Result<Router> {
    let config_with_data_dir = Config::new_with_port(config.host, config.port, data_dir);
    create_app(config_with_data_dir, network).await
}

pub async fn run_server(config: Config, network: Network) -> Result<()> {
    let app = create_app(config.clone(), network).await?;

    let bind_address = format!("{}:{}", config.host, config.port);
    info!("Starting server on {}", bind_address);

    let listener = tokio::net::TcpListener::bind(&bind_address).await?;
    info!("Server listening on {}", bind_address);

    axum::serve(listener, app).await?;
    Ok(())
}

/// Run a server with a custom data directory
pub async fn run_server_with_data_dir(
    config: Config,
    network: Network,
    data_dir: std::path::PathBuf,
) -> Result<()> {
    let config_with_data_dir = Config::new_with_port(config.host, config.port, data_dir);
    run_server(config_with_data_dir, network).await
}

/// Start a server with a random port for library usage
/// Returns the server info with the actual port used, a receiver for pool status updates, and task manager
pub async fn start_server_with_random_port(
    config: Config,
    network: Network,
) -> Result<(
    ServerInfo,
    tokio::sync::broadcast::Receiver<PoolStatus>,
    TaskManager,
)> {
    // Clone the host before moving config
    let host = config.host.clone();

    // If port is 0, the system will assign a random available port
    let config_with_random_port = Config::new_random_port(config.host, config.data_dir);

    let (app, status_receiver, task_manager) =
        create_app_with_receiver(config_with_random_port, network).await?;

    // Bind to port 0 to get a random available port
    let listener = tokio::net::TcpListener::bind(format!("{}:0", host)).await?;
    let actual_addr = listener.local_addr()?;

    let server_info = ServerInfo {
        port: actual_addr.port(),
        host: host.clone(),
    };

    info!(
        "Started server on {}:{} (random port)",
        server_info.host, server_info.port
    );

    // Start the server in a background task
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            error!("Server error: {}", e);
        }
    });

    Ok((server_info, status_receiver, task_manager))
}

/// Start a server with a random port and custom data directory for library usage
/// Returns the server info with the actual port used, a receiver for pool status updates, and task manager
pub async fn start_server_with_random_port_and_data_dir(
    config: Config,
    network: Network,
    data_dir: std::path::PathBuf,
) -> Result<(
    ServerInfo,
    tokio::sync::broadcast::Receiver<PoolStatus>,
    TaskManager,
)> {
    let config_with_data_dir = Config::new_random_port(config.host, data_dir);
    start_server_with_random_port(config_with_data_dir, network).await
}
