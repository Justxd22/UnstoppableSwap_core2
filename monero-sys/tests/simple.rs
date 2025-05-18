use monero_sys::{Daemon, SyncProgress, WalletHandle};

const PASSWORD: &str = "test";

const STAGENET_REMOTE_NODE: &str = "node.sethforprivacy.com:38089";
const STAGENET_WALLET_SEED: &str = "echo ourselves ruined oven masterful wives enough addicted future cottage illness adopt lucky movement tiger taboo imbalance antics iceberg hobby oval aloof tuesday uttered oval";
const STAGENET_WALLET_RESTORE_HEIGHT: u64 = 1728128;

#[tokio::test]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            "info,test=debug,monero_harness=debug,monero_rpc=debug,simple=trace,monero_sys=trace",
        )
        .with_test_writer()
        .init();

    let temp_dir = tempfile::tempdir().unwrap();
    let daemon = Daemon {
        address: STAGENET_REMOTE_NODE.into(),
        ssl: true,
    };

    let wallet_name = "recovered_wallet";
    let wallet_path = temp_dir.path().join(wallet_name).display().to_string();

    tracing::info!("Recovering wallet from seed");
    let wallet = WalletHandle::open_or_create_from_seed(
        wallet_path,
        STAGENET_WALLET_SEED.to_string(),
        monero::Network::Stagenet,
        STAGENET_WALLET_RESTORE_HEIGHT,
        daemon,
    )
    .await
    .expect("Failed to recover wallet");

    tracing::info!("Primary address: {}", wallet.main_address().await);

    // Wait for a while to let the wallet sync, checking sync status
    tracing::info!("Waiting for wallet to sync...");

    wallet
        .wait_until_synced(Some(|sync_progress: SyncProgress| {
            tracing::info!("Sync progress: {}%", sync_progress.percentage());
        }))
        .await
        .expect("Failed to sync wallet");

    tracing::info!("Wallet is synchronized!");

    let balance = wallet.total_balance().await;
    tracing::info!("Balance: {}", balance);

    let unlocked_balance = wallet.unlocked_balance().await;
    tracing::info!("Unlocked balance: {}", unlocked_balance);
}
