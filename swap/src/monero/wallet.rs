use crate::env::Config;
use crate::monero::{
    Amount, InsufficientFunds, PrivateViewKey, PublicViewKey, TransferProof, TxHash,
};
use ::monero::{Address, Network, PrivateKey, PublicKey};
use anyhow::{Context, Result};
use monero_rpc::wallet::{BlockHeight, MoneroWalletRpc as _, Refreshed};
use monero_rpc::{jsonrpc, wallet};
use std::future::Future;
use std::ops::Div;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Interval;
use url::Url;

#[derive(Debug)]
pub struct Wallet {
    inner: wallet::Client,
    network: Network,
    name: String,
    main_address: monero::Address,
    sync_interval: Duration,
}

impl Wallet {
    /// Connect to a wallet RPC and load the given wallet by name.
    pub async fn open_or_create(url: Url, name: String, env_config: Config) -> Result<Self> {
        let client = wallet::Client::new(url)?;

        match client.open_wallet(name.clone()).await {
            Err(error) => {
                tracing::debug!(%error, "Open wallet response error");
                client.create_wallet(name.clone(), "English".to_owned()).await.context(
                    "Unable to create Monero wallet, please ensure that the monero-wallet-rpc is available",
                )?;

                tracing::debug!(monero_wallet_name = %name, "Created Monero wallet");
            }
            Ok(_) => tracing::debug!(monero_wallet_name = %name, "Opened Monero wallet"),
        }

        Self::connect(client, name, env_config).await
    }

    /// Connects to a wallet RPC where a wallet is already loaded.
    pub async fn connect(client: wallet::Client, name: String, env_config: Config) -> Result<Self> {
        let main_address =
            monero::Address::from_str(client.get_address(0).await?.address.as_str())?;

        Ok(Self {
            inner: client,
            network: env_config.monero_network,
            name,
            main_address,
            sync_interval: env_config.monero_sync_interval(),
        })
    }

    /// Re-open the wallet using the internally stored name.
    pub async fn re_open(&self) -> Result<()> {
        self.inner.open_wallet(self.name.clone()).await?;
        Ok(())
    }

    pub async fn open(&self, filename: String) -> Result<()> {
        self.inner.open_wallet(filename).await?;
        Ok(())
    }

    /// Close the wallet and open (load) another wallet by generating it from
    /// keys. The generated wallet will remain loaded.
    pub async fn create_from_and_load(
        &self,
        file_name: String,
        private_spend_key: PrivateKey,
        private_view_key: PrivateViewKey,
        restore_height: BlockHeight,
    ) -> Result<()> {
        let public_spend_key = PublicKey::from_private_key(&private_spend_key);
        let public_view_key = PublicKey::from_private_key(&private_view_key.into());

        let address = Address::standard(self.network, public_spend_key, public_view_key);

        // Properly close the wallet before generating the other wallet to ensure that
        // it saves its state correctly
        let _ = self
            .inner
            .close_wallet()
            .await
            .context("Failed to close wallet")?;

        let _ = self
            .inner
            .generate_from_keys(
                file_name,
                address.to_string(),
                private_spend_key.to_string(),
                PrivateKey::from(private_view_key).to_string(),
                restore_height.height,
                String::from(""),
                true,
            )
            .await
            .context("Failed to generate new wallet from keys")?;

        Ok(())
    }

    /// Close the wallet and open (load) another wallet by generating it from
    /// keys. The generated wallet will be opened, all funds sweeped to the
    /// main_address and then the wallet will be re-loaded using the internally
    /// stored name.
    pub async fn create_from_keys_and_sweep(
        &self,
        file_name: String,
        private_spend_key: PrivateKey,
        private_view_key: PrivateViewKey,
        restore_height: BlockHeight,
    ) -> Result<()> {
        // Close the default wallet, generate the new wallet from the keys and load it
        self.create_from_and_load(
            file_name,
            private_spend_key,
            private_view_key,
            restore_height,
        )
        .await?;

        // Refresh the generated wallet
        if let Err(error) = self.refresh(20).await {
            return Err(anyhow::anyhow!(error)
                .context("Failed to refresh generated wallet for sweeping to default wallet"));
        }

        // Sweep all the funds from the generated wallet to the default wallet
        let sweep_result = self.inner.sweep_all(self.main_address.to_string()).await;

        match sweep_result {
            Ok(sweep_all) => {
                for tx in sweep_all.tx_hash_list {
                    tracing::info!(
                        %tx,
                        monero_address = %self.main_address,
                        "Monero transferred back to default wallet");
                }
            }
            Err(error) => {
                return Err(
                    anyhow::anyhow!(error).context("Failed to transfer Monero to default wallet")
                );
            }
        }

        let _ = self.inner.open_wallet(self.name.clone()).await?;

        Ok(())
    }

    pub async fn transfer(&self, request: TransferRequest) -> Result<TransferProof> {
        let TransferRequest {
            public_spend_key,
            public_view_key,
            amount,
        } = request;

        let destination_address =
            Address::standard(self.network, public_spend_key, public_view_key.into());

        let res = self
            .inner
            .transfer_single(0, amount.as_piconero(), &destination_address.to_string())
            .await?;

        tracing::debug!(
            %amount,
            to = %public_spend_key,
            tx_id = %res.tx_hash,
            "Successfully initiated Monero transfer"
        );

        Ok(TransferProof::new(
            TxHash(res.tx_hash),
            res.tx_key
                .context("Missing tx_key in `transfer` response")?,
        ))
    }

    pub async fn sweep_all(&self, address: Address) -> Result<Vec<TxHash>> {
        let sweep_all = self.inner.sweep_all(address.to_string()).await?;

        let tx_hashes = sweep_all.tx_hash_list.into_iter().map(TxHash).collect();
        Ok(tx_hashes)
    }

    /// Get the balance of the primary account.
    pub async fn get_balance(&self) -> Result<wallet::GetBalance> {
        Ok(self.inner.get_balance(0).await?)
    }

    pub async fn block_height(&self) -> Result<BlockHeight> {
        Ok(self.inner.get_height().await?)
    }

    pub fn get_main_address(&self) -> Address {
        self.main_address
    }

    pub async fn refresh(&self, max_attempts: usize) -> Result<Refreshed> {
        const RETRY_INTERVAL: Duration = Duration::from_secs(1);

        for i in 1..=max_attempts {
            tracing::info!(name = %self.name, attempt=i, "Syncing Monero wallet");

            let result = self.inner.refresh().await;

            match result {
                Ok(refreshed) => {
                    tracing::info!(name = %self.name, "Monero wallet synced");
                    return Ok(refreshed);
                }
                Err(error) => {
                    let attempts_left = max_attempts - i;

                    // We would not want to fail here if the height is not available
                    // as it is not critical for the operation of the wallet.
                    // We can just log a warning and continue.
                    let height = match self.inner.get_height().await {
                        Ok(height) => height.to_string(),
                        Err(_) => {
                            tracing::warn!(name = %self.name, "Failed to fetch Monero wallet height during sync");
                            "unknown".to_string()
                        }
                    };

                    tracing::warn!(attempt=i, %height, %attempts_left, name = %self.name, %error, "Failed to sync Monero wallet");

                    if attempts_left == 0 {
                        return Err(error.into());
                    }
                }
            }

            tokio::time::sleep(RETRY_INTERVAL).await;
        }
        unreachable!("Loop should have returned by now");
    }
}

/// Wait until the specified transfer has been completed or failed.
pub async fn watch_for_transfer(
    wallet: Arc<Mutex<Wallet>>,
    request: WatchRequest,
) -> Result<(), InsufficientFunds> {
    watch_for_transfer_with(wallet, request, None).await
}

/// Wait until the specified transfer has been completed or failed and listen to each new confirmation.
#[allow(clippy::too_many_arguments)]
pub async fn watch_for_transfer_with(
    wallet: Arc<Mutex<Wallet>>,
    request: WatchRequest,
    listener: Option<ConfirmationListener>,
) -> Result<(), InsufficientFunds> {
    let WatchRequest {
        conf_target,
        public_view_key,
        public_spend_key,
        transfer_proof,
        expected,
    } = request;

    let txid = transfer_proof.tx_hash();

    tracing::info!(
        %txid,
        target_confirmations = %conf_target,
        "Waiting for Monero transaction finality"
    );

    let wallet_lock = wallet.lock().await;

    let address = Address::standard(
        wallet_lock.network,
        public_spend_key,
        public_view_key.into(),
    );

    let check_interval = tokio::time::interval(wallet_lock.sync_interval.div(10));

    let wallet_name = wallet_lock.name.clone();

    let mutexed_client = Arc::new(Mutex::new(&wallet_lock.inner));

    // Make sure to release the lock before we start waiting for confimations
    let _ = wallet_lock;

    wait_for_confirmations_with(
        &wallet_lock.inner,
        transfer_proof,
        address,
        expected,
        conf_target,
        check_interval,
        wallet_name,
        listener,
    )
    .await?;

    Ok(())
}

#[derive(Debug)]
pub struct TransferRequest {
    pub public_spend_key: PublicKey,
    pub public_view_key: PublicViewKey,
    pub amount: Amount,
}

#[derive(Debug)]
pub struct WatchRequest {
    pub public_spend_key: PublicKey,
    pub public_view_key: PublicViewKey,
    pub transfer_proof: TransferProof,
    pub conf_target: u64,
    pub expected: Amount,
}

/// This is a shorthand for the dynamic type we use to pass listeners to
/// i.e. the `wait_for_confirmations` function. It is basically
/// an `async fn` which takes a `u64` and returns nothing, but in dynamic.
///
/// We use this to pass a listener that sends events to the tauri
/// frontend to show upates to the number of confirmations that
/// a tx has.
type ConfirmationListener =
    Box<dyn Fn(u64) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> + Send + 'static>;

#[allow(clippy::too_many_arguments)]
async fn wait_for_confirmations_with<
    C: monero_rpc::wallet::MoneroWalletRpc<reqwest::Client> + Sync,
>(
    client: Arc<Mutex<&C>>,
    transfer_proof: TransferProof,
    to_address: Address,
    expected: Amount,
    conf_target: u64,
    mut check_interval: Interval,
    wallet_name: String,
    listener: Option<ConfirmationListener>,
) -> Result<(), InsufficientFunds> {
    let mut seen_confirmations = 0u64;

    while seen_confirmations < conf_target {
        check_interval.tick().await; // tick() at the beginning of the loop so every `continue` tick()s as well

        let txid = transfer_proof.tx_hash().to_string();

        // Acquire a lock to the client only after awaiting the next tick.
        // This way we don't starve other tasks.
        // The lock is dropped at the end of each iteration.
        let client_lock = client.lock().await;

        let tx = match client_lock
            .check_tx_key(
                txid.clone(),
                transfer_proof.tx_key.to_string(),
                to_address.to_string(),
            )
            .await
        {
            Ok(proof) => proof,
            Err(jsonrpc::Error::JsonRpc(jsonrpc::JsonRpcError {
                code: -1,
                message,
                data,
            })) => {
                tracing::debug!(message, ?data);
                tracing::warn!(%txid, message, "`monero-wallet-rpc` failed to fetch transaction, may need to be restarted");
                continue;
            }
            // TODO: Implement this using a generic proxy for each function call once https://github.com/thomaseizinger/rust-jsonrpc-client/issues/47 is fixed.
            Err(jsonrpc::Error::JsonRpc(jsonrpc::JsonRpcError { code: -13, .. })) => {
                tracing::debug!(
                    "No wallet loaded. Opening wallet `{}` to continue monitoring of Monero transaction {}",
                    wallet_name,
                    txid
                );

                if let Err(err) = client_lock.open_wallet(wallet_name.clone()).await {
                    tracing::warn!(
                        %err,
                        "Failed to open wallet `{}` to continue monitoring of Monero transaction {}",
                        wallet_name,
                        txid
                    );
                }
                continue;
            }
            Err(other) => {
                tracing::debug!(
                    %txid,
                    "Failed to retrieve tx from blockchain: {:#}", other
                );
                continue; // treating every error as transient and retrying
                          // is obviously wrong but the jsonrpc client is
                          // too primitive to differentiate between all the
                          // cases
            }
        };

        let received = Amount::from_piconero(tx.received);

        if received != expected {
            return Err(InsufficientFunds {
                expected,
                actual: received,
            });
        }

        if tx.confirmations > seen_confirmations {
            seen_confirmations = tx.confirmations;
            tracing::info!(
                %txid,
                %seen_confirmations,
                needed_confirmations = %conf_target,
                "Received new confirmation for Monero lock tx"
            );

            // notify the listener we received new confirmations
            if let Some(listener) = &listener {
                listener(seen_confirmations).await;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracing_ext::capture_logs;
    use monero_rpc::wallet::CheckTxKey;
    use std::sync::atomic::{AtomicU32, Ordering};
    use tokio::sync::Mutex;
    use tracing::metadata::LevelFilter;

    async fn wait_for_confirmations<
        C: monero_rpc::wallet::MoneroWalletRpc<reqwest::Client> + Sync,
    >(
        client: Arc<Mutex<C>>,
        transfer_proof: TransferProof,
        to_address: Address,
        expected: Amount,
        conf_target: u64,
        check_interval: Interval,
        wallet_name: String,
    ) -> Result<(), InsufficientFunds> {
        wait_for_confirmations_with(
            client,
            transfer_proof,
            to_address,
            expected,
            conf_target,
            check_interval,
            wallet_name,
            None,
        )
        .await
    }

    #[tokio::test]
    async fn given_exact_confirmations_does_not_fetch_tx_again() {
        let client = Mutex::new(DummyClient::new(vec![Ok(CheckTxKey {
            confirmations: 10,
            received: 100,
        })]));

        let result = wait_for_confirmations(
            &*client.lock().await,
            TransferProof::new(TxHash("<FOO>".to_owned()), PrivateKey {
                scalar: crate::monero::Scalar::random(&mut rand::thread_rng())
            }),
            "53H3QthYLckeCXh9u38vohb2gZ4QgEG3FMWHNxccR6MqV1LdDVYwF1FKsRJPj4tTupWLf9JtGPBcn2MVN6c9oR7p5Uf7JdJ".parse().unwrap(),
            Amount::from_piconero(100),
            10,
            tokio::time::interval(Duration::from_millis(10)),
            "foo-wallet".to_owned(),
        )
        .await;

        assert!(result.is_ok());
        assert_eq!(
            client
                .lock()
                .await
                .check_tx_key_invocations
                .load(Ordering::SeqCst),
            1
        );
    }

    #[tokio::test]
    async fn visual_log_check() {
        let writer = capture_logs(LevelFilter::INFO);

        let client = Mutex::new(DummyClient::new(vec![
            Ok(CheckTxKey {
                confirmations: 1,
                received: 100,
            }),
            Ok(CheckTxKey {
                confirmations: 1,
                received: 100,
            }),
            Ok(CheckTxKey {
                confirmations: 1,
                received: 100,
            }),
            Ok(CheckTxKey {
                confirmations: 3,
                received: 100,
            }),
            Ok(CheckTxKey {
                confirmations: 5,
                received: 100,
            }),
        ]));

        wait_for_confirmations(
            &*client.lock().await,
            TransferProof::new(TxHash("<FOO>".to_owned()), PrivateKey {
                scalar: crate::monero::Scalar::random(&mut rand::thread_rng())
            }),
            "53H3QthYLckeCXh9u38vohb2gZ4QgEG3FMWHNxccR6MqV1LdDVYwF1FKsRJPj4tTupWLf9JtGPBcn2MVN6c9oR7p5Uf7JdJ".parse().unwrap(),
            Amount::from_piconero(100),
            5,
            tokio::time::interval(Duration::from_millis(10)),
            "foo-wallet".to_owned()
        )
        .await
        .unwrap();

        assert_eq!(
            writer.captured(),
            r" INFO swap::monero::wallet: Received new confirmation for Monero lock tx txid=<FOO> seen_confirmations=1 needed_confirmations=5
 INFO swap::monero::wallet: Received new confirmation for Monero lock tx txid=<FOO> seen_confirmations=3 needed_confirmations=5
 INFO swap::monero::wallet: Received new confirmation for Monero lock tx txid=<FOO> seen_confirmations=5 needed_confirmations=5
"
        );
    }

    #[tokio::test]
    async fn reopens_wallet_in_case_not_available() {
        let writer = capture_logs(LevelFilter::DEBUG);

        let client = Mutex::new(DummyClient::new(vec![
            Ok(CheckTxKey {
                confirmations: 1,
                received: 100,
            }),
            Ok(CheckTxKey {
                confirmations: 1,
                received: 100,
            }),
            Err((-13, "No wallet file".to_owned())),
            Ok(CheckTxKey {
                confirmations: 3,
                received: 100,
            }),
            Ok(CheckTxKey {
                confirmations: 5,
                received: 100,
            }),
        ]));

        wait_for_confirmations(
            &*client.lock().await,
            TransferProof::new(TxHash("<FOO>".to_owned()), PrivateKey {
                scalar: crate::monero::Scalar::random(&mut rand::thread_rng())
            }),
            "53H3QthYLckeCXh9u38vohb2gZ4QgEG3FMWHNxccR6MqV1LdDVYwF1FKsRJPj4tTupWLf9JtGPBcn2MVN6c9oR7p5Uf7JdJ".parse().unwrap(),
            Amount::from_piconero(100),
            5,
            tokio::time::interval(Duration::from_millis(10)),
            "foo-wallet".to_owned(),
        )
        .await
        .unwrap();

        assert_eq!(
            writer.captured(),
            r" INFO swap::monero::wallet: Received new confirmation for Monero lock tx txid=<FOO> seen_confirmations=1 needed_confirmations=5
DEBUG swap::monero::wallet: No wallet loaded. Opening wallet `foo-wallet` to continue monitoring of Monero transaction <FOO>
 INFO swap::monero::wallet: Received new confirmation for Monero lock tx txid=<FOO> seen_confirmations=3 needed_confirmations=5
 INFO swap::monero::wallet: Received new confirmation for Monero lock tx txid=<FOO> seen_confirmations=5 needed_confirmations=5
"
        );
        assert_eq!(
            client
                .lock()
                .await
                .open_wallet_invocations
                .load(Ordering::SeqCst),
            1
        );
    }

    type ErrorCode = i64;
    type ErrorMessage = String;

    struct DummyClient {
        check_tx_key_responses: Vec<Result<wallet::CheckTxKey, (ErrorCode, ErrorMessage)>>,

        check_tx_key_invocations: AtomicU32,
        open_wallet_invocations: AtomicU32,
    }

    impl DummyClient {
        fn new(
            check_tx_key_responses: Vec<Result<wallet::CheckTxKey, (ErrorCode, ErrorMessage)>>,
        ) -> Self {
            Self {
                check_tx_key_responses,
                check_tx_key_invocations: Default::default(),
                open_wallet_invocations: Default::default(),
            }
        }
    }

    #[async_trait::async_trait]
    impl monero_rpc::wallet::MoneroWalletRpc<reqwest::Client> for DummyClient {
        async fn open_wallet(
            &self,
            _: String,
        ) -> Result<wallet::WalletOpened, monero_rpc::jsonrpc::Error<reqwest::Error>> {
            self.open_wallet_invocations.fetch_add(1, Ordering::SeqCst);

            Ok(monero_rpc::wallet::Empty {})
        }

        async fn check_tx_key(
            &self,
            _: String,
            _: String,
            _: String,
        ) -> Result<wallet::CheckTxKey, monero_rpc::jsonrpc::Error<reqwest::Error>> {
            let index = self.check_tx_key_invocations.fetch_add(1, Ordering::SeqCst);

            self.check_tx_key_responses[index as usize]
                .clone()
                .map_err(|(code, message)| {
                    monero_rpc::jsonrpc::Error::JsonRpc(monero_rpc::jsonrpc::JsonRpcError {
                        code,
                        message,
                        data: None,
                    })
                })
        }

        async fn send_request<P>(
            &self,
            _: String,
        ) -> Result<monero_rpc::jsonrpc::Response<P>, reqwest::Error>
        where
            P: serde::de::DeserializeOwned,
        {
            todo!()
        }
    }
}
