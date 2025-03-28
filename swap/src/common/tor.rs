use std::path::Path;
use std::sync::Arc;

use arti_client::{
    config::{pt::TransportConfigBuilder, BridgeConfigBuilder, CfgPath, TorClientConfigBuilder},
    Error, TorClient,
};
use tor_rtcompat::tokio::TokioRustlsRuntime;

pub async fn init_tor_client(
    data_dir: &Path,
    bridges: Vec<String>,
    obfs4proxy_path: Option<String>,
) -> Result<Arc<TorClient<TokioRustlsRuntime>>, Error> {
    // We store the Tor state in the data directory
    let data_dir = data_dir.join("tor");
    let state_dir = data_dir.join("state");
    let cache_dir = data_dir.join("cache");

    // The client configuration describes how to connect to the Tor network,
    // and what directories to use for storing persistent state.
    let mut builder = TorClientConfigBuilder::from_directories(state_dir, cache_dir);

    // Add bridges
    if let Some(obfs4proxy_path) = obfs4proxy_path {
        // Add the obfs4proxy transport with the given path to the binary
        let mut value = TransportConfigBuilder::default();
        value
            .protocols(vec!["obfs4".parse().unwrap()])
            .path(CfgPath::new(obfs4proxy_path));

        builder.bridges().transports().push(value);

        for bridge_line in bridges {
            match bridge_line.parse::<BridgeConfigBuilder>() {
                Ok(bridge) => {
                    tracing::debug!(%bridge_line, "Using tor bridge");
                    builder.bridges().bridges().push(bridge);
                }
                Err(err) => {
                    tracing::error!(%err, %bridge_line, "Could not use tor bridge because we could not parse it")
                }
            }
        }
    } else if !bridges.is_empty() {
        tracing::warn!("Tor bridges cannot be used without an obfs4proxy binary");
    }

    let config = builder
        .build()
        .expect("We initialized the Tor client with all required attributes");

    // Start the Arti client, and let it bootstrap a connection to the Tor network.
    // (This takes a while to gather the necessary directory information.
    // It uses cached information when possible.)
    let runtime = TokioRustlsRuntime::current().expect("We are always running with tokio");

    tracing::debug!("Bootstrapping Tor client");

    let tor_client = TorClient::with_runtime(runtime)
        .config(config)
        .create_bootstrapped()
        .await?;

    Ok(Arc::new(tor_client))
}
