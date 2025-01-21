use crate::env::Config;
use crate::monero::{
    Amount, InsufficientFunds, PrivateViewKey, PublicViewKey, TransferProof, TxHash,
};
use ::monero::{Address, Network, PrivateKey, PublicKey};
use anyhow::{Context, Result};
use monero_c_rust::{Wallet as NativeWallet, WalletManager};
use monero_c_rust::BlockHeight;
use std::future::Future;
use std::ops::Div;
use std::path::PathBuf;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Interval;

/// Structure here:
/// Wallet has constructor functions that take a WalletManager and create a new wallet based on:
/// - a seed
/// - a wallet name

pub struct Wallet {
    manager: Arc<WalletManager>,
    inner: Mutex<NativeWallet>,
    network: Network,
    name: String,
    main_address: monero::Address,
    sync_interval: Duration,
    path: PathBuf,
}

impl Wallet {
    /// Connect to a wallet RPC and load the given wallet by name.
    pub async fn open_or_create(
        manager: Arc<WalletManager>,
        name: String,
        env_config: Config,
    ) -> Result<Self> {
        let tempDir = std::env::temp_dir();
        let name = name.clone();
        let path = tempDir.join(&name);

        if manager.as_ref().wallet_exists(path.clone())? {
            match manager.open_wallet(path.clone(), "", env_config.monero_network) {
                Err(error) => {
                    return Err(error.into());
                }
                Ok(mut wallet) => {
                    let address = wallet.get_address(0, 0)?;
                    tracing::debug!(monero_wallet_name = %name, "Opened Monero wallet");

                    return Ok(Self {
                        manager,
                        inner: Mutex::new(wallet),
                        network: env_config.monero_network,
                        name,
                        main_address: monero::Address::from_str(address.as_str())?,
                        sync_interval: env_config.monero_sync_interval(),
                        path,
                    });
                }
            }
        } else {
            tracing::debug!(monero_wallet_name = %name.clone(), "Attempted to open Monero wallet, but it does not exist. We will create it.");

            match manager.create_wallet(path.clone(), "English", "", env_config.monero_network) {
                Err(error) => {
                    tracing::error!(%error, "Failed to create Monero wallet");
                    return Err(error.into());
                }
                Ok(wallet) => {
                    tracing::info!(monero_wallet_name = %name, "Created Monero wallet");

                    let address = monero::Address::from_str(wallet.get_address(0, 0)?.as_str())?;

                    return Ok(Self {
                        manager,
                        inner: Mutex::new(wallet),
                        network: env_config.monero_network,
                        name,
                        main_address: address,
                        sync_interval: env_config.monero_sync_interval(),
                        path,
                    });
                }
            }
        }
    }

    /// Connects to a wallet RPC where a wallet is already loaded.
    // TOOD: This is needed for integration tests
    /*
    pub async fn connect(client: wallet::Client, name: String, env_config: Config) -> Result<Self> {
        let main_address =
            monero::Address::from_str(client.get_address(0).await?.address.as_str())?;

        Ok(Self {
            inner: Mutex::new(client),
            network: env_config.monero_network,
            name,
            main_address,
            sync_interval: env_config.monero_sync_interval(),
        })
    }
     */

    /// Re-open the wallet using the internally stored name.
    pub async fn re_open(&self) -> Result<()> {
        self.inner.lock().await.close_wallet()?;
        Ok(())
    }

    pub async fn open(&self, filename: PathBuf) -> Result<()> {
        self.manager.open_wallet(filename, "", self.network)?;
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

        let mut wallet = self.inner.lock().await;

        // Properly close the wallet before generating the other wallet to ensure that
        // it saves its state correctly
        let _ = wallet
            .close_wallet()
            .context("Failed to close wallet")?;

        let _ = self.manager
            .generate_from_keys(
                file_name,
                address.to_string(),
                private_spend_key.to_string(),
                PrivateKey::from(private_view_key).to_string(),
                restore_height.height,
                String::from(""),
                String::from("English"),
                self.network,
                1,
            )
            .context("Failed to generate new wallet from keys")?;

        Ok(())
    }

    /// Close the wallet and open (load) another wallet by generating it from
    /// keys. The generated wallet will be opened, all funds sweeped to the
    /// main_address and then the wallet will be re-loaded using the internally
    /// stored name.
    pub async fn create_from(
        &self,
        file_name: String,
        private_spend_key: PrivateKey,
        private_view_key: PrivateViewKey,
        restore_height: BlockHeight,
    ) -> Result<()> {
        let public_spend_key = PublicKey::from_private_key(&private_spend_key);
        let public_view_key = PublicKey::from_private_key(&private_view_key.into());

        let temp_wallet_address =
            Address::standard(self.network, public_spend_key, public_view_key);

        // Close the default wallet before generating the other wallet to ensure that
        // it saves its state correctly
        let _ = self.inner.lock().await.close_wallet()?;

        let _ = self.manager
            .generate_from_keys(
                file_name,
                temp_wallet_address.to_string(),
                private_spend_key.to_string(),
                PrivateKey::from(private_view_key).to_string(),
                restore_height.height,
                String::from(""),
                String::from("English"),
                self.network,
                1,
            )?;

        // Try to send all the funds from the generated wallet to the default wallet
        match self.refresh(3).await {
            Ok(_) => match self
                .inner
                .lock()
                .await
                .sweep_all(0, self.main_address, true)
            {
                Ok(sweep_all) => {
                    tracing::info!(
                        txid = %sweep_all.txid,
                        monero_address = %self.main_address,
                        "Monero transferred back to default wallet");
                }
                Err(error) => {
                    // TODO: Re-add the retry fix here from https://github.com/UnstoppableSwap/core/pull/254
                    tracing::warn!(
                        address = %self.main_address,
                        "Failed to transfer Monero to default wallet: {:#}", error
                    );
                }
            },
            Err(error) => {
                tracing::warn!("Failed to refresh generated wallet: {:#}", error);
            }
        }

        let _ = self
            .manager
            .open_wallet(self.path.clone(), "", self.network)?;

        Ok(())
    }

    pub async fn transfer(&self, request: TransferRequest) -> Result<TransferProof> {
        let inner = self.inner.lock().await;

        let TransferRequest {
            public_spend_key,
            public_view_key,
            amount,
        } = request;

        let destination_address =
            Address::standard(self.network, public_spend_key, public_view_key.into());

        let res = inner
            .transfer_single(0, amount.as_piconero(), &destination_address.to_string())
            .await?;

        tracing::debug!(
            %amount,
            to = %public_spend_key,
            tx_id = %res.txid,
            "Successfully initiated Monero transfer"
        );

        Ok(TransferProof::new(
            TxHash(res.txid),
            res.tx_key
                .context("Missing tx_key in `transfer` response")?,
        ))
    }

    /// Wait until the specified transfer has been completed or failed.
    pub async fn watch_for_transfer(&self, request: WatchRequest) -> Result<(), InsufficientFunds> {
        self.watch_for_transfer_with(request, None).await
    }

    /// Wait until the specified transfer has been completed or failed and listen to each new confirmation.
    #[allow(clippy::too_many_arguments)]
    pub async fn watch_for_transfer_with(
        &self,
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

        let address = Address::standard(self.network, public_spend_key, public_view_key.into());

        let check_interval = tokio::time::interval(self.sync_interval.div(10));

        wait_for_confirmations_with(
            &self.inner,
            transfer_proof,
            address,
            expected,
            conf_target,
            check_interval,
            self.name.clone(),
            listener,
        )
        .await?;

        Ok(())
    }

    pub async fn sweep_all(&self, address: Address) -> Result<Vec<TxHash>> {
        let sweep_all = self
            .inner
            .lock()
            .await
            .sweep_all(0, address, true)?;

        Ok(vec![TxHash(sweep_all.txid)])
    }

    /// Get the balance of the primary account.
    pub async fn get_balance(&self) -> Result<monero_c_rust::GetBalance> {
        Ok(self.inner.lock().await.get_balance(0)?)
    }

    pub async fn block_height(&self) -> Result<monero_c_rust::BlockHeight> {
        Ok(self.manager.get_height()?)
    }

    pub fn get_main_address(&self) -> Address {
        self.main_address
    }

    pub async fn refresh(&self, max_attempts: usize) -> Result<monero_c_rust::Refreshed> {
        const RETRY_INTERVAL: Duration = Duration::from_secs(1);

        for i in 1..=max_attempts {
            tracing::info!(name = %self.name, attempt=i, "Syncing Monero wallet");

            let result = self.inner.lock().await.refresh();

            match result {
                Ok(refreshed) => {
                    tracing::info!(name = %self.name, "Syncing Monero wallet");

                    loop {
                        let sync_height = self.inner.lock().await.get_blockchain_height()?;
                        let daemon_height = self.manager.get_height()?;

                        if sync_height >= daemon_height {
                            tracing::info!(name = %self.name, "Monero wallet synced");
                            break;
                        }

                        tracing::info!(name = %self.name, %sync_height, %daemon_height, "Syncing Monero wallet");

                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }

                    return Ok(refreshed);
                }
                Err(error) => {
                    let attempts_left = max_attempts - i;

                    tracing::warn!(attempt=i, %attempts_left, name = %self.name, %error, "Failed to sync Monero wallet");

                    if attempts_left == 0 {
                        return Err(error.into());
                    }
                }
            }

            tokio::time::sleep(RETRY_INTERVAL).await;
        }
        unreachable!("Loop should have returned by now");
    }

    pub async fn close_wallet(&self) -> Result<()> {
        self.inner.lock().await.close_wallet()?;
        Ok(())
    }
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

type ConfirmationListener =
    Box<dyn Fn(u64) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> + Send + 'static>;

#[allow(clippy::too_many_arguments)]
async fn wait_for_confirmations_with(
    client: &Mutex<monero_c_rust::Wallet>,
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
        let client = client.lock().await;

        let tx = match client.check_tx_key(
            txid.clone(),
            transfer_proof.tx_key.to_string(),
            to_address.to_string(),
        ) {
            Ok(proof) => proof,
            /*
            TODO: monero-native todo, re-implement
            Err(err) => {
                tracing::warn!(%txid, %err, "Failed to fetch Monero transaction, may need to be restarted");
                continue;
            }
            // TODO: Implement this using a generic proxy for each function call once https://github.com/thomaseizinger/rust-jsonrpc-client/issues/47 is fixed.
            Err(jsonrpc::Error::JsonRpc(jsonrpc::JsonRpcError { code: -13, .. })) => {
                tracing::debug!(
                    "Opening wallet `{}` because no wallet is loaded",
                    wallet_name
                );
                let _ = client.open_wallet(wallet_name.clone()).await;
                continue;
            }*/
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
    use tracing::metadata::LevelFilter;

    async fn wait_for_confirmations<
        C: monero_rpc::wallet::MoneroWalletRpc<reqwest::Client> + Sync,
    >(
        client: &Mutex<C>,
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
            &client,
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
            &client,
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
            &client,
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
DEBUG swap::monero::wallet: Opening wallet `foo-wallet` because no wallet is loaded
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
