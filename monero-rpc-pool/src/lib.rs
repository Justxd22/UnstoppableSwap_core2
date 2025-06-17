use std::sync::Arc;

use anyhow::Result;
use axum::{
    routing::{get, post},
    Router,
};
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;
use tracing::{error, info};

pub mod config;
pub mod database;
pub mod discovery;
// pub mod handlers;
pub mod pool;
pub mod simple_handlers;
pub mod smart_pool;

use config::Config;
use database::Database;
use discovery::NodeDiscovery;
use simple_handlers::{simple_http_handler, simple_rpc_handler, simple_stats_handler};
use smart_pool::SmartNodePool;

#[derive(Clone)]
pub struct AppState {
    pub smart_pool: Arc<RwLock<SmartNodePool>>,
}

pub async fn create_app(_config: Config) -> Result<Router> {
    // Initialize database
    let db = Database::new().await?;

    // Initialize smart pool
    let smart_pool = Arc::new(RwLock::new(SmartNodePool::new(db.clone())));

    // Initialize discovery service
    let discovery = NodeDiscovery::new(db.clone());

    // Perform initial node discovery
    info!("Performing initial node discovery...");
    if let Err(e) = discovery.discover_nodes_from_monero_fail().await {
        error!("Failed initial node discovery: {}", e);
    }

    // Start initial health check in parallel (non-blocking)
    let discovery_health_check = discovery.clone();
    tokio::spawn(async move {
        info!("Performing initial health check...");
        if let Err(e) = discovery_health_check.health_check_all_nodes().await {
            error!("Failed initial health check: {}", e);
        }
    });

    // Start periodic discovery task
    let discovery_clone = discovery.clone();
    tokio::spawn(async move {
        if let Err(e) = discovery_clone.periodic_discovery_task().await {
            error!("Periodic discovery task failed: {}", e);
        }
    });

    let app_state = AppState { smart_pool };

    let app = Router::new()
        .route("/json_rpc", post(simple_rpc_handler))
        .route("/stats", get(simple_stats_handler))
        .route("/*endpoint", get(simple_http_handler))
        .with_state(app_state)
        .layer(CorsLayer::permissive());

    Ok(app)
}

pub async fn run_server(config: Config) -> Result<()> {
    let app = create_app(config.clone()).await?;

    let bind_address = format!("{}:{}", config.host, config.port);
    info!("Starting Monero RPC Pool server on {}", bind_address);

    let listener = tokio::net::TcpListener::bind(&bind_address).await?;
    info!("Server listening on {}", bind_address);

    axum::serve(listener, app).await?;

    Ok(())
}
