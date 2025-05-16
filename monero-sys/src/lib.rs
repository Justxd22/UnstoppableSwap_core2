mod bridge;

use std::{any::Any, cmp::Ordering, ops::Deref, pin::Pin, str::FromStr};

use anyhow::{bail, Context, Result};
use cxx::let_cxx_string;
use tokio::sync::{
    mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    oneshot,
};

use bridge::ffi;

/// A handle that can communicate with the [`FfiWallet`] object.
pub struct Wallet {
    call_sender: UnboundedSender<Call>,
}

/// A wrapper around a wallet that can be used to call methods on it.
/// It must live in a single thread due to ffi constraints [1].
///
/// [1] The Monero codebase uses thread local storage and other mechanisms,
/// meaning that it's not safe to access the wallet from any thread other than
/// the one it was created on.
/// This goes for Wallet and WalletManager, meaning that each Wallet must be in its
/// WalletManager's thread (since you need a WalletManager to create a Wallet).
///
pub struct WrappedWallet {
    path: String,
    wallet: FfiWallet,
    manager: WalletManager,
    call_receiver: UnboundedReceiver<Call>,
}

/// A function call to be executed on the wallet and a channel to send the result back.
struct Call {
    function: Box<dyn FnOnce(&mut FfiWallet) -> Box<dyn Any + Send> + Send>,
    sender: oneshot::Sender<Box<dyn Any + Send>>,
}

/// A singleton responsible for managing (creating, opening, ...) wallets.
pub struct WalletManager {
    /// A wrapper around the raw C++ wallet manager pointer.
    inner: RawWalletManager,
}

/// This is our own wrapper around a raw C++ wallet manager pointer.
struct RawWalletManager {
    inner: *mut ffi::WalletManager,
}

/// A single Monero wallet.
pub struct FfiWallet {
    inner: RawWallet,
}

/// This is our own wrapper around a raw C++ wallet pointer.
struct RawWallet {
    inner: *mut ffi::Wallet,
}

/// The progress of synchronization of a wallet with the remote node.
#[derive(Debug, Clone, Copy)]
pub struct SyncProgress {
    /// The current block height of the wallet.
    pub current_block: u64,
    /// The target block height of the wallet.
    pub target_block: u64,
}

/// The status of a transaction.
pub struct TxStatus {
    /// The amount received in the transaction.
    pub received: monero::Amount,
    /// Whether the transaction is in the mempool.
    pub in_pool: bool,
    /// The number of confirmations the transaction has.
    pub confirmations: u64,
}

/// A receipt returned after successfully publishing a transaction.
/// Contains basic information needed for later verification.
pub struct TxReceipt {
    pub txid: String,
    pub tx_key: String,
    pub height: u64,
}

/// A remote node to connect to.
#[derive(Debug, Clone, Default)]
pub struct Daemon {
    pub address: String,
    pub ssl: bool,
}

/// A wrapper around a pending transaction.
pub struct PendingTransaction(*mut ffi::PendingTransaction);

impl Wallet {
    /// Execute a function on the wallet thread and return the result.
    /// Necessary because every interaction with the wallet must run on a single thread.
    pub async fn call<F, R>(&self, function: F) -> R
    where
        F: FnOnce(&mut FfiWallet) -> R + Send + 'static,
        R: Sized + Send + 'static,
    {
        // Create a oneshot channel for the result
        let (sender, receiver) = oneshot::channel();

        // Send the function call to the wallet thread (wrapped in a Box)
        self.call_sender
            .send(Call {
                function: Box::new(move |wallet| Box::new(function(wallet)) as Box<dyn Any + Send>),
                sender,
            })
            .expect("channel to be open");

        // Wait for the result and cast back to the expected type
        let result = *receiver
            .blocking_recv()
            .expect("channel to be open")
            .downcast::<R>() // We know that F returns R
            .expect("return type to be consistent");

        result
    }

    pub async fn open_or_create(
        path: String,
        daemon: Daemon,
        network: monero::Network,
    ) -> anyhow::Result<Self> {
        let (call_sender, call_receiver) = unbounded_channel();

        std::thread::spawn(move || {
            let mut manager =
                WalletManager::new(daemon.clone()).expect("wallet manager to be created");
            let wallet = manager
                .open_or_create_wallet(&path, None, network, daemon.clone())
                .expect("wallet to be created");

            let mut wrapped_wallet = WrappedWallet::new(path, wallet, manager, call_receiver);

            wrapped_wallet.run();
        });

        Ok(Wallet { call_sender })
    }

    /// Open an existing wallet or create a new one by recovering it from a
    /// mnemonic seed. If a wallet already exists at `path` it will be opened,
    /// otherwise a new wallet will be recovered using the provided seed.
    pub async fn open_or_create_from_seed(
        path: String,
        mnemonic: String,
        network: monero::Network,
        restore_height: u64,
        daemon: Daemon,
    ) -> anyhow::Result<Self> {
        let (call_sender, call_receiver) = unbounded_channel();

        // Spawn the wallet thread – all interactions with the wallet must
        // happen on the same OS thread.
        std::thread::spawn(move || {
            // Create the wallet manager in this thread first.
            let mut manager =
                WalletManager::new(daemon.clone()).expect("wallet manager to be created");

            // Decide whether we have to open an existing wallet or recover it
            // from the mnemonic.
            let wallet = if manager.wallet_exists(&path) {
                // Existing wallet – open it.
                manager
                    .open_or_create_wallet(&path, None, network, daemon.clone())
                    .expect("wallet to be opened")
            } else {
                // Wallet does not exist – recover it from the seed.
                manager
                    .recover_wallet(
                        &path,
                        None,
                        &mnemonic,
                        network,
                        restore_height,
                        daemon.clone(),
                    )
                    .expect("wallet to be recovered from seed")
            };

            let mut wrapped_wallet = WrappedWallet::new(path, wallet, manager, call_receiver);

            wrapped_wallet.run();
        });

        let mut wallet = Wallet { call_sender };
        // Make a test call to ensure that the wallet is created.
        wallet
            .main_address()
            .await
            .context("failed to create wallet")?;

        Ok(wallet)
    }

    /// Open an existing wallet or create a new one from spend/view keys. If a
    /// wallet already exists at `path` it will be opened, otherwise it will be
    /// created from the supplied keys.
    #[allow(clippy::too_many_arguments)]
    pub async fn open_or_create_from_keys(
        path: String,
        password: Option<String>,
        network: monero::Network,
        address: monero::Address,
        view_key: monero::PrivateKey,
        spend_key: monero::PrivateKey,
        restore_height: u64,
        daemon: Daemon,
    ) -> anyhow::Result<Self> {
        let (call_sender, call_receiver) = unbounded_channel();

        std::thread::spawn(move || {
            let mut manager =
                WalletManager::new(daemon.clone()).expect("wallet manager to be created");

            let wallet = manager
                .open_or_create_wallet_from_keys(
                    &path,
                    password.as_deref(),
                    network,
                    &address,
                    view_key,
                    spend_key,
                    restore_height,
                    daemon.clone(),
                )
                .expect("wallet to be opened or created from keys");

            let mut wrapped_wallet = WrappedWallet::new(path, wallet, manager, call_receiver);

            wrapped_wallet.run();
        });

        let mut wallet = Wallet { call_sender };
        // Make a test call to ensure that the wallet is created.
        wallet
            .main_address()
            .await
            .context("failed to create wallet")?;

        Ok(wallet)
    }

    pub async fn path(&self) -> String {
        self.call(move |wallet| wallet.path()).await
    }

    pub async fn main_address(&self) -> monero::Address {
        self.call(move |wallet| wallet.main_address()).await
    }

    pub async fn blockchain_height(&self) -> u64 {
        self.call(move |wallet| wallet.blockchain_height()).await
    }

    pub async fn transfer(
        &self,
        address: &monero::Address,
        amount: monero::Amount,
    ) -> anyhow::Result<TxReceipt> {
        let address = address.clone();
        self.call(move |wallet| wallet.transfer(&address, amount))
            .await
    }

    pub async fn sweep(&self, address: &monero::Address) -> anyhow::Result<Vec<String>> {
        let address = address.clone();
        self.call(move |wallet| wallet.sweep(&address)).await
    }

    pub async fn unlocked_balance(&self) -> monero::Amount {
        self.call(move |wallet| wallet.unlocked_balance()).await
    }

    pub async fn total_balance(&self) -> monero::Amount {
        self.call(move |wallet| wallet.total_balance()).await
    }

    async fn synchronized(&self) -> bool {
        self.call(move |wallet| wallet.synchronized()).await
    }

    async fn sync_progress(&self) -> SyncProgress {
        self.call(move |wallet| wallet.sync_progress()).await
    }

    pub async fn wait_until_synced(
        &self,
        listener: Option<impl Fn(SyncProgress) + Send + 'static>,
    ) -> anyhow::Result<()> {
        // We wait for ms before polling the wallet's sync status again.
        // This is ok because this doesn't involve any blocking calls.
        const POLL_INTERVAL_MILLIS: u64 = 500;

        tracing::debug!("Waiting for wallet to sync");

        // Initiate the sync (make sure to drop the lock right after)
        {
            self.call(move |wallet| {
                wallet.start_refresh();
                wallet.refresh_async();
            })
            .await;
            tracing::debug!("Wallet refresh initiated");
        }

        // Wait until the wallet is connected to the daemon.
        loop {
            let connected = self.call(move |wallet| wallet.connected()).await;

            if connected {
                break;
            }

            tracing::trace!(
                "Wallet not connected to daemon, sleeping for {}ms",
                POLL_INTERVAL_MILLIS
            );

            tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MILLIS)).await;
        }

        // Keep track of the sync progress to avoid calling
        // the listener twice with the same progress
        let mut current_progress = SyncProgress::zero();

        // Continue polling until the sync is complete
        loop {
            // Get the current sync status (releasing the lock immediately afterwords)
            let (synced, sync_progress) =
                { (self.synchronized().await, self.sync_progress().await) };

            // Notify the listener (if it exists)
            if sync_progress > current_progress {
                if let Some(listener) = &listener {
                    listener(sync_progress);
                }
            }

            // Update the current progress
            current_progress = sync_progress;

            // If the wallet is synced, break out of the loop.
            if synced {
                break;
            }

            tracing::trace!(
                "Wallet sync not complete, sleeping for {}ms",
                POLL_INTERVAL_MILLIS
            );

            // Otherwise, sleep for a bit and try again.
            tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MILLIS)).await;
        }

        tracing::debug!("Wallet synced");

        Ok(())
    }

    async fn check_tx_status(
        &self,
        txid: String,
        tx_key: monero::PrivateKey,
        destination_address: &monero::Address,
    ) -> anyhow::Result<TxStatus> {
        let destination_address = destination_address.clone();
        self.call(move |wallet| wallet.check_tx_status(&txid, tx_key, &destination_address))
            .await
    }

    pub async fn wait_until_confirmed(
        &self,
        txid: String,
        tx_key: monero::PrivateKey,
        destination_address: &monero::Address,
        expected_amount: monero::Amount,
        confirmations: u64,
        listener: Option<impl Fn(u64) + Send + 'static>,
    ) -> anyhow::Result<()> {
        const DEFAULT_CHECK_INTERVAL_SECS: u64 = 15;

        let mut poll_interval = tokio::time::interval(tokio::time::Duration::from_secs(
            DEFAULT_CHECK_INTERVAL_SECS,
        ));

        loop {
            poll_interval.tick().await;

            let tx_status = match self
                .check_tx_status(txid.clone(), tx_key, destination_address)
                .await
            {
                Ok(tx_status) => tx_status,
                Err(e) => {
                    tracing::error!(
                        "Failed to check tx status: {}, rechecking in {}s",
                        e,
                        DEFAULT_CHECK_INTERVAL_SECS
                    );
                    continue;
                }
            };

            // Make sure the amount is correct
            if tx_status.received != expected_amount {
                tracing::error!(
                    "Transaction received amount mismatch: expected {}, got {}",
                    expected_amount,
                    tx_status.received
                );
                return Err(anyhow::anyhow!(
                    "Transaction received amount mismatch: expected {}, got {}",
                    expected_amount,
                    tx_status.received
                ));
            }

            // If the listener exists, notify it of the result
            if let Some(listener) = &listener {
                listener(tx_status.confirmations);
            }

            // Stop when we have the required number of confirmations
            if tx_status.confirmations >= confirmations {
                break;
            }
        }

        // Signal success
        Ok(())
    }
}

impl WrappedWallet {
    fn new(
        path: String,
        wallet: FfiWallet,
        manager: WalletManager,
        call_receiver: UnboundedReceiver<Call>,
    ) -> Self {
        Self {
            path,
            wallet,
            manager,
            call_receiver,
        }
    }

    fn run(&mut self) {
        while let Some(call) = self.call_receiver.blocking_recv() {
            let result = (call.function)(&mut self.wallet);
            call.sender
                .send(result)
                .expect("failed to send result back to caller");
        }
    }
}

impl Drop for WrappedWallet {
    fn drop(&mut self) {
        if let Err(e) = self.manager.close_wallet(&mut self.wallet) {
            tracing::error!("Failed to close wallet: {}", e);
            // If we fail to close the wallet, we can't do anything about it.
            // This results in it being leaked.
        }
        // TODO: dispose of the manager
    }
}

impl WalletManager {
    /// For now we don't support custom difficulty
    const DEFAULT_KDF_ROUNDS: u64 = 1;

    /// Get the wallet manager instance.
    /// You can optionally pass a daemon with which the wallet manager and
    /// all wallets opened by the manager will connect.
    pub fn new(daemon: Daemon) -> anyhow::Result<Self> {
        // Install the log callback to route c++ logs to tracing.
        bridge::log::install_log_callback();

        let manager = ffi::getWalletManager();
        if manager.is_null() {
            bail!("Failed to get wallet manager, got null pointer");
        }
        let mut manager = Self {
            inner: RawWalletManager::new(manager),
        };

        manager.set_daemon_address(&daemon.address);

        Ok(manager)
    }

    /// Create a new wallet, or open if it already exists.
    pub fn open_or_create_wallet(
        &mut self,
        path: &str,
        password: Option<&str>,
        network: monero::Network,
        daemon: Daemon,
    ) -> anyhow::Result<FfiWallet> {
        // If we haven't loaded the wallet, but it already exists, open it.
        if self.wallet_exists(path) {
            tracing::debug!(wallet=%path, "Wallet already exists, opening it");

            return self
                .open_wallet(path, password, network, daemon)
                .context(format!("Failed to open wallet `{}`", &path));
        }

        tracing::debug!(%path, "Wallet doesn't exist, creating it");

        // Otherwise, create (and open) a new wallet.
        let kdf_rounds = Self::DEFAULT_KDF_ROUNDS;
        let_cxx_string!(path = path);
        let_cxx_string!(password = password.unwrap_or(""));
        let_cxx_string!(language = "English");
        let network_type = network.into();

        let wallet_pointer =
            self.inner
                .pinned()
                .createWallet(&path, &password, &language, network_type, kdf_rounds);

        if wallet_pointer.is_null() {
            anyhow::bail!("Failed to create wallet, got null pointer");
        }

        let raw_wallet = RawWallet::new(wallet_pointer);
        let wallet = FfiWallet::new(raw_wallet, daemon)
            .context(format!("Failed to initialize wallet `{}`", &path))?;

        Ok(wallet)
    }

    /// Create a new wallet from keys or open if it already exists.
    #[allow(clippy::too_many_arguments)]
    pub fn open_or_create_wallet_from_keys(
        &mut self,
        path: &str,
        password: Option<&str>,
        network: monero::Network,
        address: &monero::Address,
        view_key: monero::PrivateKey,
        spend_key: monero::PrivateKey,
        restore_height: u64,
        daemon: Daemon,
    ) -> Result<FfiWallet> {
        if self.wallet_exists(path) {
            tracing::info!(wallet=%path, "Wallet already exists, opening it");

            self.open_wallet(path, password, network, daemon.clone())
                .context(format!("Failed to open wallet `{}`", &path))?;
        }

        let_cxx_string!(path = path);
        let_cxx_string!(password = password.unwrap_or(""));
        let_cxx_string!(language = "English");
        let network_type = network.into();
        let_cxx_string!(address = address.to_string());
        let_cxx_string!(view_key = view_key.to_string());
        let_cxx_string!(spend_key = spend_key.to_string());
        let kdf_rounds = Self::DEFAULT_KDF_ROUNDS;

        let wallet_pointer = self.inner.pinned().createWalletFromKeys(
            &path,
            &password,
            &language,
            network_type,
            restore_height,
            &address,
            &view_key,
            &spend_key,
            kdf_rounds,
        );

        if wallet_pointer.is_null() {
            anyhow::bail!("Failed to create wallet from keys, got null pointer");
        }

        let raw_wallet = RawWallet::new(wallet_pointer);
        let wallet = FfiWallet::new(raw_wallet, daemon)
            .context(format!("Failed to initialize wallet `{}`", &path))?;

        Ok(wallet)
    }

    /// Recover a wallet from a mnemonic seed (electrum seed).
    #[allow(clippy::too_many_arguments)]
    pub fn recover_wallet(
        &mut self,
        path: &str,
        password: Option<&str>,
        mnemonic: &str,
        network: monero::Network,
        restore_height: u64,
        daemon: Daemon,
    ) -> anyhow::Result<FfiWallet> {
        let_cxx_string!(path = path);
        let_cxx_string!(password = password.unwrap_or(""));
        let_cxx_string!(mnemonic = mnemonic);
        let_cxx_string!(seed_offset = "");

        let network_type = network.into();
        let wallet_pointer = self.inner.pinned().recoveryWallet(
            &path,
            &password,
            &mnemonic,
            network_type,
            restore_height,
            Self::DEFAULT_KDF_ROUNDS,
            &seed_offset,
        );

        let raw_wallet = RawWallet::new(wallet_pointer);
        let wallet = FfiWallet::new(raw_wallet, daemon)
            .context(format!("Failed to initialize wallet `{}`", &path))?;

        Ok(wallet)
    }

    /// Close a wallet, storing the wallet state.
    fn close_wallet(&mut self, wallet: &mut FfiWallet) -> anyhow::Result<()> {
        // Safety: we know we have a valid, unique pointer to the wallet
        let success = unsafe { self.inner.pinned().closeWallet(wallet.inner.inner, true) };

        if !success {
            anyhow::bail!("Failed to close wallet");
        }

        Ok(())
    }

    /// Open a wallet. Only used internally. Use [`WalletManager::open_or_create_wallet`] instead.
    ///
    /// Todo: add listener support?
    fn open_wallet(
        &mut self,
        path: &str,
        password: Option<&str>,
        network_type: monero::Network,
        daemon: Daemon,
    ) -> anyhow::Result<FfiWallet> {
        let_cxx_string!(path = path);
        let_cxx_string!(password = password.unwrap_or(""));
        let network_type = network_type.into();
        let kdf_rounds = Self::DEFAULT_KDF_ROUNDS;

        let wallet_pointer = unsafe {
            self.inner.pinned().openWallet(
                &path,
                &password,
                network_type,
                kdf_rounds,
                std::ptr::null_mut(),
            )
        };

        if wallet_pointer.is_null() {
            anyhow::bail!("Failed to open wallet: got null pointer")
        }

        let raw_wallet = RawWallet::new(wallet_pointer);

        let wallet = FfiWallet::new(raw_wallet, daemon).context("Failed to initialize wallet")?;

        Ok(wallet)
    }

    /// Set the address of the remote node ("daemon").
    fn set_daemon_address(&mut self, address: &str) {
        tracing::debug!(%address, "Updating wallet manager's remote node");

        let_cxx_string!(address = address);

        self.inner.pinned().setDaemonAddress(&address);
    }

    /// Check if a wallet exists at the given path.
    pub fn wallet_exists(&mut self, path: &str) -> bool {
        let_cxx_string!(path = path);
        self.inner.pinned().walletExists(&path)
    }

    /// Check if the wallet manager is connected to the configured daemon.
    ///
    /// The manager might takes a few seconds to connect to the daemon after startup.
    pub fn connected(&mut self) -> bool {
        let mut version = 0;
        unsafe { self.inner.pinned().connected(&mut version) }
    }

    /// Get the current blockchain height, if the manager is connected to a daemon.
    ///
    /// Returns None if the manager is not connected to a daemon.
    pub fn blockchain_height(&mut self) -> Option<u64> {
        match self.inner.pinned().blockchainHeight() {
            0 => None,
            height => Some(height),
        }
    }
}

impl RawWalletManager {
    fn new(inner: *mut ffi::WalletManager) -> Self {
        Self { inner }
    }

    /// Get a pinned reference to the inner (c++) wallet manager.
    /// This is a convenience function necessary because
    /// the ffi interface mostly takes a Pin<&mut T> but
    /// we haven't figured out how to hold that in the struct.
    pub fn pinned(&mut self) -> Pin<&mut ffi::WalletManager> {
        unsafe {
            Pin::new_unchecked(
                self.inner
                    .as_mut()
                    .expect("wallet manager pointer not to be null"),
            )
        }
    }
}

impl FfiWallet {
    const MAIN_ACCOUNT_INDEX: u32 = 0;

    /// Create and initialize new wallet from a raw C++ wallet pointer.
    fn new(inner: RawWallet, daemon: Daemon) -> anyhow::Result<Self> {
        if inner.inner.is_null() {
            anyhow::bail!("Failed to create wallet: got null pointer");
        }

        let mut wallet = Self { inner: inner };

        tracing::debug!("Initializing wallet");

        wallet.check_error()?;

        let daemon = daemon;

        wallet
            .init(&daemon.address, daemon.ssl)
            .context("Failed to initialize wallet")?;
        wallet.check_error()?;

        wallet.start_refresh();
        wallet.check_error()?;

        Ok(wallet)
    }

    /// Get the path to the wallet file.
    pub fn path(&self) -> String {
        ffi::walletPath(&*self.inner).to_string()
    }

    /// Get the address for the given account and address index.
    /// address(0, 0) is the main address.
    /// We don't use anything besides the main address so this is a private method (for now).
    fn address(&self, account_index: u32, address_index: u32) -> monero::Address {
        let address = ffi::address(&*self.inner, account_index, address_index);
        monero::Address::from_str(&address.to_string()).expect("wallet's own address to be valid")
    }

    fn set_daemon_address(&mut self, address: &str) -> anyhow::Result<()> {
        let_cxx_string!(address = address);
        let raw_wallet = &mut self.inner;

        let success = ffi::setWalletDaemon(raw_wallet.pinned(), &address);

        if !success {
            self.check_error().context("Failed to set daemon address")?;
            anyhow::bail!("Failed to set daemon address");
        }

        Ok(())
    }

    /// Get the main address of the walllet (account 0, address 0).
    pub fn main_address(&self) -> monero::Address {
        self.address(Self::MAIN_ACCOUNT_INDEX, 0)
    }

    /// Initialize the wallet and download initial values from the remote node.
    /// Does not actuallyt sync the wallet, use any of the refresh methods to do that.
    fn init(&mut self, daemon_address: &str, ssl: bool) -> anyhow::Result<()> {
        let_cxx_string!(daemon_address = daemon_address);
        let_cxx_string!(daemon_username = "");
        let_cxx_string!(daemon_password = "");
        let_cxx_string!(proxy_address = "");

        let raw_wallet = &mut self.inner;

        let success = raw_wallet.pinned().init(
            &daemon_address,
            0,
            &daemon_username,
            &daemon_password,
            ssl,
            true,
            &proxy_address,
        );

        if !success {
            self.check_error().context("Failed to initialize wallet")?;
            anyhow::bail!("Failed to initialize wallet, error string empty");
        }

        Ok(())
    }

    /// Get the sync progress of the wallet as a percentage.
    ///
    /// Returns a zeroed sync progress if the daemon is not connected.
    fn sync_progress(&self) -> SyncProgress {
        let current_block = self.inner.blockChainHeight();
        let target_block = self.daemon_blockchain_height().unwrap_or(0);

        if target_block == 0 {
            return SyncProgress::zero();
        }

        SyncProgress::new(current_block, target_block)
    }

    fn connected(&self) -> bool {
        match self.inner.connected() {
            ffi::ConnectionStatus::Connected => true,
            ffi::ConnectionStatus::WrongVersion => {
                tracing::warn!("Version mismatch with daemon");
                false
            }
            ffi::ConnectionStatus::Disconnected => false,
            // Fallback since C++ allows any other value.
            status => {
                tracing::error!("Unknown connection status: `{}`", status.repr);
                false
            }
        }
    }

    /// Start the background refresh thread (refreshes every 10 seconds).
    fn start_refresh(&mut self) {
        self.inner.pinned().startRefresh();
    }

    /// Refresh the wallet asynchronously.
    /// Same as start_refresh except that the background thread only
    /// refreshes once. Maybe?
    fn refresh_async(&mut self) {
        self.inner.pinned().refreshAsync();
    }

    /// Get the current blockchain height.
    fn blockchain_height(&self) -> u64 {
        self.inner.blockChainHeight()
    }

    /// Get the daemon's blockchain height.
    ///
    /// Returns the height of the blockchain, if connected.
    /// Returns None if not connected.
    fn daemon_blockchain_height(&self) -> Option<u64> {
        // Here we actually use the _target_ height -- incase the remote node is
        // currently catching up we want to work with the height it ends up at.
        match self.inner.daemonBlockChainTargetHeight() {
            0 => None,
            height => Some(height),
        }
    }

    /// Get the total balance across all accounts.
    fn total_balance(&self) -> monero::Amount {
        let balance = self.inner.balanceAll();
        monero::Amount::from_pico(balance)
    }

    /// Get the total unlocked balance across all accounts in atomic units.
    fn unlocked_balance(&self) -> monero::Amount {
        let balance = self.inner.unlockedBalanceAll();
        monero::Amount::from_pico(balance)
    }

    /// Check if the wallet is synced with the daemon.
    fn synchronized(&self) -> bool {
        self.inner.synchronized()
    }

    /// Set the allow mismatched daemon version flag.
    ///
    /// This is needed for regnet compatibility.
    ///
    /// _Do not use for anything besides testing._
    fn allow_mismatched_daemon_version(&mut self) {
        self.inner.pinned().setAllowMismatchedDaemonVersion(true);
    }

    /// Check the status of a transaction.
    fn check_tx_status(
        &mut self,
        txid: &str,
        tx_key: monero::PrivateKey,
        address: &monero::Address,
    ) -> anyhow::Result<TxStatus> {
        let_cxx_string!(txid = txid);
        let_cxx_string!(tx_key = tx_key.to_string());
        let_cxx_string!(address = address.to_string());

        let mut received = 0;
        let mut in_pool = false;
        let mut confirmations = 0;

        let raw_wallet = &mut self.inner;

        let success = ffi::checkTxKey(
            raw_wallet.pinned(),
            &txid,
            &tx_key,
            &address,
            &mut received,
            &mut in_pool,
            &mut confirmations,
        );

        if !success {
            self.check_error().context("Failed to check tx key")?;
            anyhow::bail!("Failed to check tx key");
        }

        Ok(TxStatus {
            received: monero::Amount::from_pico(received),
            in_pool,
            confirmations,
        })
    }

    /// Transfer a specified amount of monero to a specified address and return a receipt containing
    /// the transaction id, transaction key and current blockchain height. This can be used later
    /// to prove the transfer or to wait for confirmations.
    fn transfer(
        &mut self,
        address: &monero::Address,
        amount: monero::Amount,
    ) -> anyhow::Result<TxReceipt> {
        let_cxx_string!(address = address.to_string());
        let amount = amount.as_pico();

        // First we need to create a pending transaction.
        let mut pending_tx = PendingTransaction(ffi::createTransaction(
            self.inner.pinned(),
            &address,
            amount,
        ));

        // Get the txid from the pending transaction before we publish,
        // otherwise it might be null.
        let txid = ffi::pendingTransactionTxId(&pending_tx) // UniquePtr<CxxString>
            .to_string();

        // Publish the transaction
        let result = pending_tx
            .publish()
            .context("Failed to publish transaction");

        // Check for errors (make sure to dispose the transaction)
        if result.is_err() {
            self.dispose_transaction(pending_tx);
            bail!("Failed to publish transaction");
        }

        // Fetch the tx key from the wallet.
        let_cxx_string!(txid_cxx = txid.clone());
        let tx_key = ffi::walletGetTxKey(&*self.inner, &txid_cxx).to_string();

        // Get current blockchain height (wallet height).
        let height = self.blockchain_height();

        // Dispose the pending transaction object to avoid memory leak.
        self.dispose_transaction(pending_tx);

        Ok(TxReceipt {
            txid,
            tx_key,
            height,
        })
    }

    /// Sweep all funds from the wallet to a specified address.
    /// Returns a list of transaction ids of the created transactions.
    fn sweep(&mut self, address: &monero::Address) -> anyhow::Result<Vec<String>> {
        let_cxx_string!(address = address.to_string());

        // Create the sweep transaction
        let mut pending_tx =
            PendingTransaction(ffi::createSweepTransaction(self.inner.pinned(), &address));

        // Get the txids from the pending transaction before we publish,
        // otherwise it might be null.
        let txids: Vec<String> = ffi::pendingTransactionTxIds(&pending_tx)
            .into_iter()
            .map(|s| s.to_string())
            .collect();

        // Publish the transaction
        let result = pending_tx
            .publish()
            .context("Failed to publish transaction");

        // Dispose of the transaction to avoid leaking memory.
        self.dispose_transaction(pending_tx);

        result.map(|_| txids)
    }

    /// Dispose (deallocate) a pending transaction object.
    /// Always call this before dropping a pending transaction object,
    /// otherwise we leak memory.
    fn dispose_transaction(&mut self, tx: PendingTransaction) {
        unsafe { self.inner.pinned().disposeTransaction(tx.0) };
    }

    /// Return `Ok` when the wallet is ok, otherwise return the error.
    /// This is a convenience method we use for retrieving errors after
    /// a method call failed.
    ///
    /// We have to pass the raw wallet here to make sure we don't have to
    /// release the mutex in between an operation and the check.
    fn check_error(&self) -> anyhow::Result<()> {
        let mut status = 0;
        let mut error_string = String::new();
        let_cxx_string!(error_string_ref = &mut error_string);

        self.inner
            .statusWithErrorString(&mut status, error_string_ref);

        // If the status is ok, we return None
        if status == 0 {
            return Ok(());
        }

        let error_string = if error_string.is_empty() {
            "unknown error, error not set".to_string()
        } else {
            error_string
        };

        let error_type = if status == 2 { "critical" } else { "error" };

        // Otherwise we return the error
        bail!(format!(
            "Experienced wallet error ({}): `{}`",
            error_type,
            error_string.to_string()
        ))
    }
}

/// Safety: We check that it's never accessed outside the homethread at runtime.
unsafe impl Send for RawWalletManager {}

impl PendingTransaction {
    /// Return `Ok` when the pending transaction is ok, otherwise return the error.
    /// This is a convenience method we use for retrieving errors after
    /// a method call failed.
    fn check_error(&self) -> anyhow::Result<()> {
        let status = self.status();
        let error_string = ffi::pendingTransactionErrorString(self);

        if status == 0 {
            return Ok(());
        }

        let error_type = if status == 2 { "critical" } else { "error" };

        bail!(format!(
            "Experienced pending transaction error ({}): {}",
            error_type, error_string
        ))
    }

    /// Publish this transaction to the blockchain or return an error.
    ///
    /// **Important**: you still have to dispose the transaction.
    fn publish(&mut self) -> anyhow::Result<()> {
        self.check_error().context("Failed to create transaction")?;

        // Then we commit it to the blockchain.
        let_cxx_string!(filename = ""); // Empty filename means we commit to the blockchain
        let success = self.pinned().commit(&filename, false);

        if success {
            Ok(())
        } else {
            // Get the error from the pending transaction.
            Err(self
                .check_error()
                .context("Failed to commit transaction to blockchain")
                .err()
                .unwrap_or(anyhow::anyhow!(
                    "Failed to commit transaction to blockchain"
                )))
        }
    }
}

impl SyncProgress {
    /// Create a new sync progress object.
    fn new(current_block: u64, target_block: u64) -> Self {
        Self {
            current_block,
            target_block,
        }
    }

    /// Create a new sync progress object with zero progess.
    fn zero() -> Self {
        Self {
            current_block: 0,
            target_block: 1,
        }
    }

    /// Get the sync progress as a fraction.
    pub fn fraction(&self) -> f32 {
        self.current_block as f32 / self.target_block as f32
    }

    /// Get the sync progress as a percentage.
    pub fn percentage(&self) -> f32 {
        100.0 * self.fraction()
    }
}

impl PartialOrd for SyncProgress {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.fraction().partial_cmp(&other.fraction())
    }
}

impl PartialEq for SyncProgress {
    fn eq(&self, other: &Self) -> bool {
        self.fraction() == other.fraction()
    }
}

/// Safety: We check that it's never accessed outside the homethread at runtime.
unsafe impl Send for RawWallet {}

impl RawWallet {
    fn new(inner: *mut ffi::Wallet) -> Self {
        Self { inner }
    }

    /// Convenience method for getting a pinned reference to the inner (c++) wallet.
    fn pinned(&mut self) -> Pin<&mut ffi::Wallet> {
        unsafe { Pin::new_unchecked(self.inner.as_mut().expect("wallet pointer not to be null")) }
    }
}

// We implement Deref for RawWallet such that we can use the
// const c++ methods directly on the RawWallet struct.
impl Deref for RawWallet {
    type Target = ffi::Wallet;

    fn deref(&self) -> &ffi::Wallet {
        unsafe { self.inner.as_ref().expect("wallet pointer not to be null") }
    }
}

impl PendingTransaction {
    fn pinned(&mut self) -> Pin<&mut ffi::PendingTransaction> {
        unsafe {
            Pin::new_unchecked(
                self.0
                    .as_mut()
                    .expect("pending transaction pointer not to be null"),
            )
        }
    }
}

impl Deref for PendingTransaction {
    type Target = ffi::PendingTransaction;

    fn deref(&self) -> &ffi::PendingTransaction {
        unsafe {
            self.0
                .as_ref()
                .expect("pending transaction pointer not to be null")
        }
    }
}
