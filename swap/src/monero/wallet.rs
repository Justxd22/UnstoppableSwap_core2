//! This module contains the [`Wallets`] struct, which we use to manage and access the
//! Monero blockchain and wallets.
//!
//! Mostly we do two things:
//!  - wait for transactions to be confirmed
//!  - send money from one wallet to another.

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use monero::{Address, Network};
pub use monero_sys::{Daemon, WalletHandle as Wallet};
use uuid::Uuid;

use super::{BlockHeight, TransferProof, TxHash};

/// Entrance point to the Monero blockchain.
/// You can use this struct to open specific wallets and monitor the blockchain.
pub struct Wallets {
    wallet_dir: PathBuf,
    network: Network,
    daemon: Daemon,
    main_wallet: Arc<Wallet>,
    /// Whether we're running in regtest mode.
    /// Since Network::Regtest isn't a thing we have to use an extra flag.
    /// When we're in regtest mode, we need to unplug some safty nets to make the Wallet work.
    regtest: bool,
}

/// A request to watch for a transfer.
pub struct WatchRequest {
    pub public_view_key: super::PublicViewKey,
    pub public_spend_key: monero::PublicKey,
    /// The proof of the transfer.
    pub transfer_proof: TransferProof,
    /// The expected amount of the transfer.
    pub expected_amount: monero::Amount,
    /// The number of confirmations required for the transfer to be considered confirmed.
    pub confirmation_target: u64,
}

/// Transfer a specified amount of money to a specified address.
pub struct TransferRequest {
    pub public_spend_key: monero::PublicKey,
    pub public_view_key: super::PublicViewKey,
    pub amount: monero::Amount,
}

impl Wallets {
    /// Create a new `Wallets` instance.
    /// Wallets will be opened on the specified network, connected to the specified daemon
    /// and stored in the specified directory.
    ///
    /// The main wallet will be kept alive and synced, other wallets are
    /// opened and closed on demand.
    pub async fn new(
        wallet_dir: PathBuf,
        main_wallet_name: String,
        daemon: Daemon,
        network: Network,
        regtest: bool,
    ) -> Result<Self> {
        let main_wallet = Wallet::open_or_create(
            wallet_dir.join(&main_wallet_name).display().to_string(),
            daemon.clone(),
            network,
            true,
        )
        .await
        .context("Failed to open main wallet")?;

        if regtest {
            main_wallet.unsafe_prepare_for_regtest().await;
        }

        let mut wallets = HashMap::new();
        let main_wallet = Arc::new(main_wallet);
        wallets.insert(main_wallet_name.clone(), Arc::downgrade(&main_wallet));

        let wallets = Self {
            wallet_dir,
            network,
            daemon,
            main_wallet,
            regtest,
        };

        Ok(wallets)
    }

    /// Open the lock wallet of a specific swap.
    /// Used to redeem (Bob) or refund (Alice) the Monero.
    pub async fn swap_wallet(
        &self,
        swap_id: Uuid,
        spend_key: monero::PrivateKey,
        view_key: super::PrivateViewKey,
        tx_lock_id: TxHash,
    ) -> Result<Arc<Wallet>> {
        // Derive wallet address from the keys
        let address = {
            let public_spend_key = monero::PublicKey::from_private_key(&spend_key);
            let public_view_key = monero::PublicKey::from_private_key(&view_key.into());

            monero::Address::standard(self.network, public_spend_key, public_view_key)
        };

        // The wallet's filename is just the swap's uuid as a string
        let filename = swap_id.to_string();
        let wallet_path = self.wallet_dir.join(&filename).display().to_string();

        let blockheight = self
            .main_wallet
            .blockchain_height()
            .await
            .context("Couldn't fetch blockchain height")?;

        let wallet = Wallet::open_or_create_from_keys(
            wallet_path.clone(),
            None,
            self.network,
            address,
            view_key.into(),
            spend_key,
            blockheight,
            false, // We don't sync the swap wallet, just import the transaction
            self.daemon.clone(),
        )
        .await
        .context(format!(
            "Failed to open or create wallet `{}` from the specified keys",
            wallet_path
        ))?;

        if self.regtest {
            wallet.unsafe_prepare_for_regtest().await;
        }

        tracing::debug!(
            %swap_id,
            "Opened temporary Monero wallet, loading lock transaction"
        );

        wallet
            .scan_transaction(tx_lock_id.0.clone())
            .await
            .context("Couldn't import Monero lock transaction")?;

        Ok(Arc::new(wallet))
    }

    /// Get the main wallet (specified when initializing the `Wallets` instance).
    pub async fn main_wallet(&self) -> Arc<Wallet> {
        self.main_wallet.clone()
    }

    /// Get the current blockchain height.
    /// May fail if not connected to a daemon.
    pub async fn blockchain_height(&self) -> Result<BlockHeight> {
        let wallet = self.main_wallet().await;

        Ok(BlockHeight {
            height: wallet.blockchain_height().await.context(
                "Failed to get blockchain height: wallet manager not connected to daemon",
            )?,
        })
    }

    /// Wait until a transfer is detected and confirmed.
    ///
    /// You can pass a listener function that will be called with
    /// the current number of confirmations every time we check the blockchain.
    /// This means that it may be called multiple times with the same number of confirmations.
    pub async fn wait_until_confirmed(
        &self,
        watch_request: WatchRequest,
        listener: Option<impl Fn(u64) + Send + 'static>,
    ) -> Result<()> {
        let wallet = self.main_wallet().await;

        let address = Address::standard(
            self.network,
            watch_request.public_spend_key,
            watch_request.public_view_key.0,
        );

        wallet
            .wait_until_confirmed(
                watch_request.transfer_proof.tx_hash.0.clone(),
                watch_request.transfer_proof.tx_key,
                &address,
                watch_request.expected_amount,
                watch_request.confirmation_target,
                listener,
            )
            .await?;

        Ok(())
    }

    pub async fn block_height(&self) -> Result<BlockHeight> {
        Ok(BlockHeight {
            height: self.main_wallet.blockchain_height().await.context(
                "Failed to get blockchain height",
            )?,
        })
    }
}

impl TransferRequest {
    pub fn address_and_amount(&self, network: Network) -> (Address, monero::Amount) {
        (
            Address::standard(network, self.public_spend_key, self.public_view_key.0),
            self.amount,
        )
    }
}

/// Pass this to [`Wallet::wait_until_confirmed`] or [`Wallet::wait_until_synced`]
/// to not receive any confirmation callbacks.
pub fn no_listener<T>() -> Option<impl Fn(T) + Send + 'static> {
    Some(|_| {})
}

// use crate::env::Config;
// use crate::monero::{
//     Amount, InsufficientFunds, PrivateViewKey, PublicViewKey, TransferProof, TxHash,
// };
// use ::monero::{Address, Network, PrivateKey, PublicKey};
// use anyhow::{Context, Result};
// use monero_rpc::wallet::{BlockHeight, MoneroWalletRpc as _, Refreshed};
// use monero_rpc::{jsonrpc, wallet};
// use std::future::Future;
// use std::ops::Div;
// use std::pin::Pin;
// use std::str::FromStr;
// use std::sync::Arc;
// use std::time::Duration;
// use tokio::sync::Mutex;
// use tokio::time::Interval;
// use url::Url;

// /// This is our connection to the monero blockchain which we use
// /// all over the codebase, mostly as `Arc<Mutex<Wallet>>`.
// ///
// /// It represents a connection to a monero-wallet-rpc daemon,
// /// which can load a (single) wallet at a time.
// /// This struct contains methods for opening, closing, creating
// /// wallet and for sending funds from the loaded wallet.
// #[derive(Debug)]
// pub struct Wallet<C = wallet::Client> {
//     inner: C,
//     network: Network,
//     /// The file name of the main wallet (the first wallet loaded)
//     main_wallet: String,
//     /// The first address of the main wallet
//     main_address: monero::Address,
//     sync_interval: Duration,
// }

// impl Wallet {
//     /// Connect to a wallet RPC and load the given wallet by name.
//     pub async fn open_or_create(url: Url, name: String, env_config: Config) -> Result<Self> {
//         let client = wallet::Client::new(url)?;

//         match client.open_wallet(name.clone()).await {
//             Err(error) => {
//                 tracing::debug!(%error, "Failed to open wallet, trying to create instead");

//                 client.create_wallet(name.clone(), "English".to_owned()).await.context(
//                     "Unable to create Monero wallet, please ensure that the monero-wallet-rpc is available",
//                 )?;

//                 tracing::debug!(monero_wallet_name = %name, "Created Monero wallet");
//             }
//             Ok(_) => tracing::debug!(monero_wallet_name = %name, "Opened Monero wallet"),
//         }

//         Self::connect(client, name, env_config).await
//     }

//     /// Connects to a wallet RPC where a wallet is already loaded.
//     pub async fn connect(client: wallet::Client, name: String, env_config: Config) -> Result<Self> {
//         let main_address =
//             monero::Address::from_str(client.get_address(0).await?.address.as_str())?;

//         Ok(Self {
//             inner: client,
//             network: env_config.monero_network,
//             main_wallet: name,
//             main_address,
//             sync_interval: env_config.monero_sync_interval(),
//         })
//     }

//     /// This can be used to create dummy wallet for testing purposes.
//     /// Warning: filled with non-sense values, don't use for anything
//     /// but as a wrapper around your dummy client.
//     #[cfg(test)]
//     fn from_dummy<T: monero_rpc::wallet::MoneroWalletRpc<reqwest::Client> + Sync>(
//         client: T,
//         network: Network,
//     ) -> Wallet<T> {
//         // Here we make up some values just so we can use the wallet in tests
//         // Todo: verify this works
//         use curve25519_dalek::scalar::Scalar;

//         let privkey = PrivateKey::from_scalar(Scalar::one());
//         let pubkey = PublicKey::from_private_key(&privkey);

//         Wallet {
//             inner: client,
//             network,
//             sync_interval: Duration::from_secs(100),
//             main_wallet: "foo".into(),
//             main_address: Address::standard(network, pubkey, pubkey),
//         }
//     }

//     /// Re-open the internally stored wallet from it's file.
//     pub async fn re_open(&self) -> Result<()> {
//         self.open(self.main_wallet.clone())
//             .await
//             .context("Failed to re-open main wallet")?;

//         Ok(())
//     }

//     /// Open a monero wallet from a file.
//     pub async fn open(&self, filename: String) -> Result<()> {
//         self.inner
//             .open_wallet(filename)
//             .await
//             .context("Failed to open ")?;
//         Ok(())
//     }

//     /// Close the wallet and open (load) another wallet by generating it from
//     /// keys. The generated wallet will remain loaded.
//     ///
//     /// If the wallet already exists, it will just be loaded instead.
//     pub async fn open_or_create_from_keys(
//         &self,
//         file_name: String,
//         private_spend_key: PrivateKey,
//         private_view_key: PrivateViewKey,
//         restore_height: BlockHeight,
//     ) -> Result<()> {
//         let public_spend_key = PublicKey::from_private_key(&private_spend_key);
//         let public_view_key = PublicKey::from_private_key(&private_view_key.into());

//         let address = Address::standard(self.network, public_spend_key, public_view_key);

//         // Properly close the wallet before generating the other wallet to ensure that
//         // it saves its state correctly
//         let _ = self
//             .inner
//             .close_wallet()
//             .await
//             .context("Failed to close wallet")?;

//         let result = self
//             .inner
//             .generate_from_keys(
//                 file_name.clone(),
//                 address.to_string(),
//                 private_spend_key.to_string(),
//                 PrivateKey::from(private_view_key).to_string(),
//                 restore_height.height,
//                 String::from(""),
//                 true,
//             )
//             .await;

//         // If we failed to create the wallet because it already exists,
//         // we just try to open it instead
//         match result {
//             Ok(_) => Ok(()),
//             Err(error) if error.to_string().contains("Wallet already exists") => {
//                 tracing::debug!(
//                     monero_wallet_name = &file_name,
//                     "Cannot create wallet because it already exists, loading instead"
//                 );

//                 self.open(file_name)
//                     .await
//                     .context("Failed to create wallet from keys ('Wallet already exists'), subsequent attempt to open failed, too")
//             }
//             Err(error) => Err(error).context("Failed to create wallet from keys"),
//         }
//     }

//     /// A wrapper around [`create_from_keys_and_sweep_to`] that sweeps all funds to the
//     /// main address of the wallet.
//     /// For the ASB this is the main wallet.
//     /// For the CLI, I don't know which wallet it is.
//     ///
//     /// Returns the tx hashes of the sweep.
//     pub async fn create_from_keys_and_sweep(
//         &self,
//         file_name: String,
//         private_spend_key: PrivateKey,
//         private_view_key: PrivateViewKey,
//         restore_height: BlockHeight,
//     ) -> Result<Vec<TxHash>> {
//         self.create_from_keys_and_sweep_to(
//             file_name,
//             private_spend_key,
//             private_view_key,
//             restore_height,
//             self.main_address,
//         )
//         .await
//     }

//     /// Close the wallet and open (load) another wallet by generating it from
//     /// keys. The generated wallet will be opened, all funds sweeped to the
//     /// specified destination address and then the original wallet will be re-loaded using the internally
//     /// stored name.
//     ///
//     /// Returns the tx hashes of the sweep.
//     pub async fn create_from_keys_and_sweep_to(
//         &self,
//         file_name: String,
//         private_spend_key: PrivateKey,
//         private_view_key: PrivateViewKey,
//         restore_height: BlockHeight,
//         destination_address: Address,
//     ) -> Result<Vec<TxHash>> {
//         // Close the default wallet, generate the new wallet from the keys and load it
//         self.open_or_create_from_keys(
//             file_name,
//             private_spend_key,
//             private_view_key,
//             restore_height,
//         )
//         .await?;

//         // Refresh the generated wallet
//         self.refresh(20)
//             .await
//             .context("Failed to refresh generated wallet for sweeping to destination address")?;

//         // Sweep all the funds from the generated wallet to the specified destination address
//         let sweep_result = self
//             .sweep_all(destination_address)
//             .await
//             .context("Failed to transfer Monero to destination address")?;

//         for tx in &sweep_result {
//             tracing::info!(
//                 %tx,
//                 monero_address = %destination_address,
//                 "Monero transferred to destination address");
//         }

//         self.re_open().await?;

//         Ok(sweep_result)
//     }

//     /// Transfer a specified amount of monero to a specified address.
//     pub async fn transfer(&self, request: TransferRequest) -> Result<TransferProof> {
//         let TransferRequest {
//             public_spend_key,
//             public_view_key,
//             amount,
//         } = request;

//         let destination_address =
//             Address::standard(self.network, public_spend_key, public_view_key.into());

//         let res = self
//             .inner
//             .transfer_single(0, amount.as_piconero(), &destination_address.to_string())
//             .await?;

//         tracing::debug!(
//             %amount,
//             to = %public_spend_key,
//             tx_id = %res.tx_hash,
//             "Successfully initiated Monero transfer"
//         );

//         Ok(TransferProof::new(
//             TxHash(res.tx_hash),
//             res.tx_key
//                 .context("Missing tx_key in `transfer` response")?,
//         ))
//     }

//     /// Send all funds from the currently loaded wallet to a specified address.
//     pub async fn sweep_all(&self, address: Address) -> Result<Vec<TxHash>> {
//         let sweep_all = self.inner.sweep_all(address.to_string()).await?;

//         let tx_hashes = sweep_all.tx_hash_list.into_iter().map(TxHash).collect();
//         Ok(tx_hashes)
//     }

//     /// Get the balance of the primary account.
//     pub async fn get_balance(&self) -> Result<wallet::GetBalance> {
//         Ok(self.inner.get_balance(0).await?)
//     }

//     pub async fn block_height(&self) -> Result<BlockHeight> {
//         Ok(self.inner.get_height().await?)
//     }

//     pub fn get_main_address(&self) -> Address {
//         self.main_address
//     }

//     pub async fn refresh(&self, max_attempts: usize) -> Result<Refreshed> {
//         const RETRY_INTERVAL: Duration = Duration::from_secs(1);

//         for i in 1..=max_attempts {
//             tracing::info!(name = %self.main_wallet, attempt=i, "Syncing Monero wallet");

//             let result = self.inner.refresh().await;

//             match result {
//                 Ok(refreshed) => {
//                     tracing::info!(name = %self.main_wallet, "Monero wallet synced");
//                     return Ok(refreshed);
//                 }
//                 Err(error) => {
//                     let attempts_left = max_attempts - i;

//                     // We would not want to fail here if the height is not available
//                     // as it is not critical for the operation of the wallet.
//                     // We can just log a warning and continue.
//                     let height = match self.inner.get_height().await {
//                         Ok(height) => height.to_string(),
//                         Err(_) => {
//                             tracing::warn!(name = %self.main_wallet, "Failed to fetch Monero wallet height during sync");
//                             "unknown".to_string()
//                         }
//                     };

//                     tracing::warn!(attempt=i, %height, %attempts_left, name = %self.main_wallet, %error, "Failed to sync Monero wallet");

//                     if attempts_left == 0 {
//                         return Err(error.into());
//                     }
//                 }
//             }

//             tokio::time::sleep(RETRY_INTERVAL).await;
//         }
//         unreachable!("Loop should have returned by now");
//     }
// }

// /// Wait until the specified transfer has been completed or failed.
// pub async fn watch_for_transfer(
//     wallet: Arc<Mutex<Wallet>>,
//     request: WatchRequest,
// ) -> Result<(), InsufficientFunds> {
//     watch_for_transfer_with(wallet, request, None).await
// }

// /// Wait until the specified transfer has been completed or failed and listen to each new confirmation.
// #[allow(clippy::too_many_arguments)]
// pub async fn watch_for_transfer_with(
//     wallet: Arc<Mutex<Wallet>>,
//     request: WatchRequest,
//     listener: Option<ConfirmationListener>,
// ) -> Result<(), InsufficientFunds> {
//     let WatchRequest {
//         conf_target,
//         public_view_key,
//         public_spend_key,
//         transfer_proof,
//         expected,
//     } = request;

//     let txid = transfer_proof.tx_hash();

//     tracing::info!(
//         %txid,
//         target_confirmations = %conf_target,
//         "Waiting for Monero transaction finality"
//     );

//     let address = Address::standard(
//         wallet.lock().await.network,
//         public_spend_key,
//         public_view_key.into(),
//     );

//     let check_interval = tokio::time::interval(wallet.lock().await.sync_interval.div(10));
//     let wallet_name = wallet.lock().await.main_wallet.clone();

//     wait_for_confirmations_with(
//         wallet.clone(),
//         transfer_proof,
//         address,
//         expected,
//         conf_target,
//         check_interval,
//         wallet_name,
//         listener,
//     )
//     .await?;

//     Ok(())
// }

// #[derive(Debug)]
// pub struct TransferRequest {
//     pub public_spend_key: PublicKey,
//     pub public_view_key: PublicViewKey,
//     pub amount: Amount,
// }

// #[derive(Debug)]
// pub struct WatchRequest {
//     pub public_spend_key: PublicKey,
//     pub public_view_key: PublicViewKey,
//     pub transfer_proof: TransferProof,
//     pub conf_target: u64,
//     pub expected: Amount,
// }

// /// This is a shorthand for the dynamic type we use to pass listeners to
// /// i.e. the `wait_for_confirmations` function. It is basically
// /// an `async fn` which takes a `u64` and returns nothing, but in dynamic.
// ///
// /// We use this to pass a listener that sends events to the tauri
// /// frontend to show upates to the number of confirmations that
// /// a tx has.
// type ConfirmationListener =
//     Box<dyn Fn(u64) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> + Send + 'static>;

// #[allow(clippy::too_many_arguments)]
// async fn wait_for_confirmations_with<
//     C: monero_rpc::wallet::MoneroWalletRpc<reqwest::Client> + Sync,
// >(
//     wallet: Arc<Mutex<Wallet<C>>>,
//     transfer_proof: TransferProof,
//     to_address: Address,
//     expected: Amount,
//     conf_target: u64,
//     mut check_interval: Interval,
//     wallet_name: String,
//     listener: Option<ConfirmationListener>,
// ) -> Result<(), InsufficientFunds> {
//     let mut seen_confirmations = 0u64;

//     while seen_confirmations < conf_target {
//         check_interval.tick().await; // tick() at the beginning of the loop so every `continue` tick()s as well

//         let txid = transfer_proof.tx_hash().to_string();

//         // Make sure to drop the lock before matching on the result
//         // otherwise it will deadlock on the error code -13 case
//         let result = wallet
//             .lock()
//             .await
//             .inner
//             .check_tx_key(
//                 txid.clone(),
//                 transfer_proof.tx_key.to_string(),
//                 to_address.to_string(),
//             )
//             .await;

//         let tx = match result {
//             Ok(proof) => proof,
//             Err(jsonrpc::Error::JsonRpc(jsonrpc::JsonRpcError {
//                 code: -1,
//                 message,
//                 data,
//             })) => {
//                 tracing::debug!(message, ?data);
//                 tracing::warn!(%txid, message, "`monero-wallet-rpc` failed to fetch transaction, may need to be restarted");
//                 continue;
//             }
//             // TODO: Implement this using a generic proxy for each function call once https://github.com/thomaseizinger/rust-jsonrpc-client/issues/47 is fixed.
//             Err(jsonrpc::Error::JsonRpc(jsonrpc::JsonRpcError { code: -13, .. })) => {
//                 tracing::debug!(
//                     "No wallet loaded. Opening wallet `{}` to continue monitoring of Monero transaction {}",
//                     wallet_name,
//                     txid
//                 );

//                 if let Err(err) = wallet
//                     .lock()
//                     .await
//                     .inner
//                     .open_wallet(wallet_name.clone())
//                     .await
//                 {
//                     tracing::warn!(
//                         %err,
//                         "Failed to open wallet `{}` to continue monitoring of Monero transaction {}",
//                         wallet_name,
//                         txid
//                     );
//                 }
//                 continue;
//             }
//             Err(other) => {
//                 tracing::debug!(
//                     %txid,
//                     "Failed to retrieve tx from blockchain: {:#}", other
//                 );
//                 continue; // treating every error as transient and retrying
//                           // is obviously wrong but the jsonrpc client is
//                           // too primitive to differentiate between all the
//                           // cases
//             }
//         };

//         let received = Amount::from_piconero(tx.received);

//         if received != expected {
//             return Err(InsufficientFunds {
//                 expected,
//                 actual: received,
//             });
//         }

//         if tx.confirmations > seen_confirmations {
//             seen_confirmations = tx.confirmations;
//             tracing::info!(
//                 %txid,
//                 %seen_confirmations,
//                 needed_confirmations = %conf_target,
//                 "Received new confirmation for Monero lock tx"
//             );

//             // notify the listener we received new confirmations
//             if let Some(listener) = &listener {
//                 listener(seen_confirmations).await;
//             }
//         }
//     }

//     Ok(())
// }

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use crate::tracing_ext::capture_logs;
//     use monero_rpc::wallet::CheckTxKey;
//     use std::sync::atomic::{AtomicU32, Ordering};
//     use tokio::sync::Mutex;
//     use tracing::metadata::LevelFilter;

//     async fn wait_for_confirmations<
//         C: monero_rpc::wallet::MoneroWalletRpc<reqwest::Client> + Sync,
//     >(
//         client: Arc<Mutex<Wallet<C>>>,
//         transfer_proof: TransferProof,
//         to_address: Address,
//         expected: Amount,
//         conf_target: u64,
//         check_interval: Interval,
//         wallet_name: String,
//     ) -> Result<(), InsufficientFunds> {
//         wait_for_confirmations_with(
//             client,
//             transfer_proof,
//             to_address,
//             expected,
//             conf_target,
//             check_interval,
//             wallet_name,
//             None,
//         )
//         .await
//     }

//     #[tokio::test]
//     async fn given_exact_confirmations_does_not_fetch_tx_again() {
//         let wallet = Arc::new(Mutex::new(Wallet::from_dummy(
//             DummyClient::new(vec![Ok(CheckTxKey {
//                 confirmations: 10,
//                 received: 100,
//             })]),
//             Network::Testnet,
//         )));

//         let result = wait_for_confirmations(
//             wallet.clone(),
//             TransferProof::new(TxHash("<FOO>".to_owned()), PrivateKey {
//                 scalar: crate::monero::Scalar::random(&mut rand::thread_rng())
//             }),
//             "53H3QthYLckeCXh9u38vohb2gZ4QgEG3FMWHNxccR6MqV1LdDVYwF1FKsRJPj4tTupWLf9JtGPBcn2MVN6c9oR7p5Uf7JdJ".parse().unwrap(),
//             Amount::from_piconero(100),
//             10,
//             tokio::time::interval(Duration::from_millis(10)),
//             "foo-wallet".to_owned(),
//         )
//         .await;

//         assert!(result.is_ok());
//         assert_eq!(
//             wallet
//                 .lock()
//                 .await
//                 .inner
//                 .check_tx_key_invocations
//                 .load(Ordering::SeqCst),
//             1
//         );
//     }

//     #[tokio::test]
//     async fn visual_log_check() {
//         let writer = capture_logs(LevelFilter::INFO);

//         let client = Arc::new(Mutex::new(Wallet::from_dummy(
//             DummyClient::new(vec![
//                 Ok(CheckTxKey {
//                     confirmations: 1,
//                     received: 100,
//                 }),
//                 Ok(CheckTxKey {
//                     confirmations: 1,
//                     received: 100,
//                 }),
//                 Ok(CheckTxKey {
//                     confirmations: 1,
//                     received: 100,
//                 }),
//                 Ok(CheckTxKey {
//                     confirmations: 3,
//                     received: 100,
//                 }),
//                 Ok(CheckTxKey {
//                     confirmations: 5,
//                     received: 100,
//                 }),
//             ]),
//             Network::Testnet,
//         )));

//         wait_for_confirmations(
//             client.clone(),
//             TransferProof::new(TxHash("<FOO>".to_owned()), PrivateKey {
//                 scalar: crate::monero::Scalar::random(&mut rand::thread_rng())
//             }),
//             "53H3QthYLckeCXh9u38vohb2gZ4QgEG3FMWHNxccR6MqV1LdDVYwF1FKsRJPj4tTupWLf9JtGPBcn2MVN6c9oR7p5Uf7JdJ".parse().unwrap(),
//             Amount::from_piconero(100),
//             5,
//             tokio::time::interval(Duration::from_millis(10)),
//             "foo-wallet".to_owned()
//         )
//         .await
//         .unwrap();

//         assert_eq!(
//             writer.captured(),
//             r" INFO swap::monero::wallet: Received new confirmation for Monero lock tx txid=<FOO> seen_confirmations=1 needed_confirmations=5
//  INFO swap::monero::wallet: Received new confirmation for Monero lock tx txid=<FOO> seen_confirmations=3 needed_confirmations=5
//  INFO swap::monero::wallet: Received new confirmation for Monero lock tx txid=<FOO> seen_confirmations=5 needed_confirmations=5
// "
//         );
//     }

//     #[tokio::test]
//     async fn reopens_wallet_in_case_not_available() {
//         let writer = capture_logs(LevelFilter::DEBUG);

//         let client = Arc::new(Mutex::new(Wallet::from_dummy(
//             DummyClient::new(vec![
//                 Ok(CheckTxKey {
//                     confirmations: 1,
//                     received: 100,
//                 }),
//                 Ok(CheckTxKey {
//                     confirmations: 1,
//                     received: 100,
//                 }),
//                 Err((-13, "No wallet file".to_owned())),
//                 Ok(CheckTxKey {
//                     confirmations: 3,
//                     received: 100,
//                 }),
//                 Ok(CheckTxKey {
//                     confirmations: 5,
//                     received: 100,
//                 }),
//             ]),
//             Network::Testnet,
//         )));

//         tokio::time::timeout(Duration::from_secs(30), wait_for_confirmations(
//             client.clone(),
//             TransferProof::new(TxHash("<FOO>".to_owned()), PrivateKey {
//                 scalar: crate::monero::Scalar::random(&mut rand::thread_rng())
//             }),
//             "53H3QthYLckeCXh9u38vohb2gZ4QgEG3FMWHNxccR6MqV1LdDVYwF1FKsRJPj4tTupWLf9JtGPBcn2MVN6c9oR7p5Uf7JdJ".parse().unwrap(),
//             Amount::from_piconero(100),
//             5,
//             tokio::time::interval(Duration::from_millis(10)),
//             "foo-wallet".to_owned(),
//         ))
//         .await
//         .expect("timeout: shouldn't take more than 10 seconds")
//         .unwrap();

//         assert_eq!(
//             writer.captured(),
//             r" INFO swap::monero::wallet: Received new confirmation for Monero lock tx txid=<FOO> seen_confirmations=1 needed_confirmations=5
// DEBUG swap::monero::wallet: No wallet loaded. Opening wallet `foo-wallet` to continue monitoring of Monero transaction <FOO>
//  INFO swap::monero::wallet: Received new confirmation for Monero lock tx txid=<FOO> seen_confirmations=3 needed_confirmations=5
//  INFO swap::monero::wallet: Received new confirmation for Monero lock tx txid=<FOO> seen_confirmations=5 needed_confirmations=5
// "
//         );
//         assert_eq!(
//             client
//                 .lock()
//                 .await
//                 .inner
//                 .open_wallet_invocations
//                 .load(Ordering::SeqCst),
//             1
//         );
//     }

//     type ErrorCode = i64;
//     type ErrorMessage = String;

//     struct DummyClient {
//         check_tx_key_responses: Vec<Result<wallet::CheckTxKey, (ErrorCode, ErrorMessage)>>,

//         check_tx_key_invocations: AtomicU32,
//         open_wallet_invocations: AtomicU32,
//     }

//     impl DummyClient {
//         fn new(
//             check_tx_key_responses: Vec<Result<wallet::CheckTxKey, (ErrorCode, ErrorMessage)>>,
//         ) -> Self {
//             Self {
//                 check_tx_key_responses,
//                 check_tx_key_invocations: Default::default(),
//                 open_wallet_invocations: Default::default(),
//             }
//         }
//     }

//     #[async_trait::async_trait]
//     impl monero_rpc::wallet::MoneroWalletRpc<reqwest::Client> for DummyClient {
//         async fn open_wallet(
//             &self,
//             _: String,
//         ) -> Result<wallet::WalletOpened, monero_rpc::jsonrpc::Error<reqwest::Error>> {
//             self.open_wallet_invocations.fetch_add(1, Ordering::SeqCst);

//             Ok(monero_rpc::wallet::Empty {})
//         }

//         async fn check_tx_key(
//             &self,
//             _: String,
//             _: String,
//             _: String,
//         ) -> Result<wallet::CheckTxKey, monero_rpc::jsonrpc::Error<reqwest::Error>> {
//             let index = self.check_tx_key_invocations.fetch_add(1, Ordering::SeqCst);

//             self.check_tx_key_responses[index as usize]
//                 .clone()
//                 .map_err(|(code, message)| {
//                     monero_rpc::jsonrpc::Error::JsonRpc(monero_rpc::jsonrpc::JsonRpcError {
//                         code,
//                         message,
//                         data: None,
//                     })
//                 })
//         }

//         async fn send_request<P>(
//             &self,
//             _: String,
//         ) -> Result<monero_rpc::jsonrpc::Response<P>, reqwest::Error>
//         where
//             P: serde::de::DeserializeOwned,
//         {
//             todo!()
//         }
//     }
// }
