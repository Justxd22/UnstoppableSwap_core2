#![warn(
    unused_extern_crates,
    missing_copy_implementations,
    rust_2018_idioms,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::fallible_impl_from,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::dbg_macro
)]
#![forbid(unsafe_code)]
#![allow(non_snake_case)]

use anyhow::{bail, Context, Result};
use comfy_table::Table;
use libp2p::Swarm;
use std::convert::TryInto;
use std::env;
use std::sync::Arc;
use structopt::clap;
use structopt::clap::ErrorKind;
use swap::asb::command::{parse_args, Arguments, Command};
use swap::asb::config::{
    initial_setup, query_user_for_initial_config, read_config, Config, ConfigNotInitialized,
};
use swap::asb::{cancel, punish, redeem, refund, safely_abort, EventLoop, Finality, KrakenRate};
use swap::common::tor::init_tor_client;
use swap::common::tracing_util::Format;
use swap::common::{self, get_logs, warn_if_outdated};
use swap::database::{open_db, AccessMode};
use swap::network::rendezvous::XmrBtcNamespace;
use swap::network::swarm;
use swap::protocol::alice::{run, AliceState};
use swap::seed::Seed;
use swap::{bitcoin, kraken, monero};
use tracing_subscriber::filter::LevelFilter;

const DEFAULT_WALLET_NAME: &str = "asb-wallet";

#[tokio::main]
pub async fn main() -> Result<()> {
    let Arguments {
        testnet,
        json,
        config_path,
        env_config,
        cmd,
    } = match parse_args(env::args_os()) {
        Ok(args) => args,
        Err(e) => {
            // make sure to display the clap error message it exists
            if let Some(clap_err) = e.downcast_ref::<clap::Error>() {
                if let ErrorKind::HelpDisplayed | ErrorKind::VersionDisplayed = clap_err.kind {
                    println!("{}", clap_err.message);
                    std::process::exit(0);
                }
            }
            bail!(e);
        }
    };

    // Check in the background if there's a new version available
    tokio::spawn(async move { warn_if_outdated(env!("CARGO_PKG_VERSION")).await });

    // Read config from the specified path
    let config = match read_config(config_path.clone())? {
        Ok(config) => config,
        Err(ConfigNotInitialized {}) => {
            initial_setup(config_path.clone(), query_user_for_initial_config(testnet)?)?;
            read_config(config_path)?.expect("after initial setup config can be read")
        }
    };

    // Initialize tracing
    let format = if json { Format::Json } else { Format::Raw };
    let log_dir = config.data.dir.join("logs");
    common::tracing_util::init(LevelFilter::DEBUG, format, log_dir, None)
        .expect("initialize tracing");

    // Check for conflicting env / config values
    if config.monero.network != env_config.monero_network {
        bail!(format!(
            "Expected monero network in config file to be {:?} but was {:?}",
            env_config.monero_network, config.monero.network
        ));
    }
    if config.bitcoin.network != env_config.bitcoin_network {
        bail!(format!(
            "Expected bitcoin network in config file to be {:?} but was {:?}",
            env_config.bitcoin_network, config.bitcoin.network
        ));
    }

    let seed =
        Seed::from_file_or_generate(&config.data.dir).expect("Could not retrieve/initialize seed");

    match cmd {
        Command::Start { resume_only } => {
            let db = open_db(config.data.dir.join("sqlite"), AccessMode::ReadWrite, None).await?;

            // check and warn for duplicate rendezvous points
            let mut rendezvous_addrs = config.network.rendezvous_point.clone();
            let prev_len = rendezvous_addrs.len();
            rendezvous_addrs.sort();
            rendezvous_addrs.dedup();
            let new_len = rendezvous_addrs.len();

            if new_len < prev_len {
                tracing::warn!(
                    "`rendezvous_point` config has {} duplicate entries, they are being ignored.",
                    prev_len - new_len
                );
            }

            // Initialize Monero wallet
            let monero_wallet = init_monero_wallet(&config, env_config).await?;
            let monero_address = monero_wallet.get_main_address();
            tracing::info!(%monero_address, "Monero wallet address");

            // Check Monero balance
            let monero = monero_wallet.get_balance().await?;
            match (monero.balance, monero.unlocked_balance) {
                (0, _) => {
                    tracing::warn!(
                        %monero_address,
                        "The Monero balance is 0, make sure to deposit funds at",
                    )
                }
                (total, 0) => {
                    let total = monero::Amount::from_piconero(total);
                    tracing::warn!(
                        %total,
                        "Unlocked Monero balance is 0, total balance is",
                    )
                }
                (total, unlocked) => {
                    let total = monero::Amount::from_piconero(total);
                    let unlocked = monero::Amount::from_piconero(unlocked);
                    tracing::info!(%total, %unlocked, "Monero wallet balance");
                }
            }

            // Initialize Bitcoin wallet
            let bitcoin_wallet = init_bitcoin_wallet(&config, &seed, env_config).await?;
            let bitcoin_balance = bitcoin_wallet.balance().await?;
            tracing::info!(%bitcoin_balance, "Bitcoin wallet balance");

            // Connect to Kraken
            let kraken_price_updates = kraken::connect(config.maker.price_ticker_ws_url.clone())?;

            let kraken_rate = KrakenRate::new(config.maker.ask_spread, kraken_price_updates);
            let namespace = XmrBtcNamespace::from_is_testnet(testnet);

            // Initialize Tor client
            let tor_client = init_tor_client(&config.data.dir).await?.into();

            let (mut swarm, onion_addresses) = swarm::asb(
                &seed,
                config.maker.min_buy_btc,
                config.maker.max_buy_btc,
                kraken_rate.clone(),
                resume_only,
                env_config,
                namespace,
                &rendezvous_addrs,
                tor_client,
            )?;

            for listen in config.network.listen.clone() {
                if let Err(e) = Swarm::listen_on(&mut swarm, listen.clone()) {
                    tracing::warn!("Failed to listen on network interface {}: {}. Consider removing it from the config.", listen, e);
                }
            }

            if config.tor.register_hidden_service {
                for onion_address in onion_addresses {
                    // We need to sleep here to wait for the bootstrap process to start BEFORE instructing libp2p to listen on the onion address
                    // This is a temporary workaround but if we don't do this it does not work
                    tracing::info!("Waiting for 5s to allow onion service bootstrapping to start");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

                    match swarm.listen_on(onion_address.clone()) {
                        Err(e) => {
                            tracing::warn!(
                                "Failed to listen on onion address {}: {}",
                                onion_address,
                                e
                            );
                        }
                        _ => {
                            swarm.add_external_address(onion_address);
                        }
                    }
                }
            }

            tracing::info!(peer_id = %swarm.local_peer_id(), "Network layer initialized");

            for external_address in config.network.external_addresses {
                swarm.add_external_address(external_address);
            }

            let (event_loop, mut swap_receiver) = EventLoop::new(
                swarm,
                env_config,
                Arc::new(bitcoin_wallet),
                Arc::new(monero_wallet),
                db,
                kraken_rate.clone(),
                config.maker.min_buy_btc,
                config.maker.max_buy_btc,
                config.maker.external_bitcoin_redeem_address,
            )
            .unwrap();

            tokio::spawn(async move {
                while let Some(swap) = swap_receiver.recv().await {
                    let rate = kraken_rate.clone();
                    tokio::spawn(async move {
                        let swap_id = swap.swap_id;
                        match run(swap, rate).await {
                            Ok(state) => {
                                tracing::debug!(%swap_id, final_state=%state, "Swap completed")
                            }
                            Err(error) => {
                                tracing::error!(%swap_id, "Swap failed: {:#}", error)
                            }
                        }
                    });
                }
            });

            event_loop.run().await;
        }
        Command::History => {
            let db = open_db(config.data.dir.join("sqlite"), AccessMode::ReadOnly, None).await?;

            let mut table = Table::new();

            table.set_header(vec!["SWAP ID", "STATE"]);

            for (swap_id, state) in db.all().await? {
                let state: AliceState = state.try_into()?;
                table.add_row(vec![swap_id.to_string(), state.to_string()]);
            }

            println!("{}", table);
        }
        Command::Config => {
            let config_json = serde_json::to_string_pretty(&config)?;
            println!("{}", config_json);
        }
        Command::Logs {
            logs_dir,
            swap_id,
            redact,
        } => {
            let dir = logs_dir.unwrap_or(config.data.dir.join("logs"));

            let log_messages = get_logs(dir, swap_id, redact).await?;

            for msg in log_messages {
                println!("{msg}");
            }
        }
        Command::WithdrawBtc { amount, address } => {
            let bitcoin_wallet = init_bitcoin_wallet(&config, &seed, env_config).await?;

            let amount = match amount {
                Some(amount) => amount,
                None => {
                    bitcoin_wallet
                        .max_giveable(address.script_pubkey().len())
                        .await?
                }
            };

            let psbt = bitcoin_wallet
                .send_to_address(address, amount, None)
                .await?;
            let signed_tx = bitcoin_wallet.sign_and_finalize(psbt).await?;

            bitcoin_wallet.broadcast(signed_tx, "withdraw").await?;
        }
        Command::Balance => {
            let monero_wallet = init_monero_wallet(&config, env_config).await?;
            let monero_balance = monero_wallet.get_balance().await?;
            tracing::info!(%monero_balance);

            let bitcoin_wallet = init_bitcoin_wallet(&config, &seed, env_config).await?;
            let bitcoin_balance = bitcoin_wallet.balance().await?;
            tracing::info!(%bitcoin_balance);
            tracing::info!(%bitcoin_balance, %monero_balance, "Current balance");
        }
        Command::Cancel { swap_id } => {
            let db = open_db(config.data.dir.join("sqlite"), AccessMode::ReadWrite, None).await?;

            let bitcoin_wallet = init_bitcoin_wallet(&config, &seed, env_config).await?;

            let (txid, _) = cancel(swap_id, Arc::new(bitcoin_wallet), db).await?;

            tracing::info!("Cancel transaction successfully published with id {}", txid);
        }
        Command::Refund { swap_id } => {
            let db = open_db(config.data.dir.join("sqlite"), AccessMode::ReadWrite, None).await?;

            let bitcoin_wallet = init_bitcoin_wallet(&config, &seed, env_config).await?;
            let monero_wallet = init_monero_wallet(&config, env_config).await?;

            refund(
                swap_id,
                Arc::new(bitcoin_wallet),
                Arc::new(monero_wallet),
                db,
            )
            .await?;

            tracing::info!("Monero successfully refunded");
        }
        Command::Punish { swap_id } => {
            let db = open_db(config.data.dir.join("sqlite"), AccessMode::ReadWrite, None).await?;

            let bitcoin_wallet = init_bitcoin_wallet(&config, &seed, env_config).await?;

            let (txid, _) = punish(swap_id, Arc::new(bitcoin_wallet), db).await?;

            tracing::info!("Punish transaction successfully published with id {}", txid);
        }
        Command::SafelyAbort { swap_id } => {
            let db = open_db(config.data.dir.join("sqlite"), AccessMode::ReadWrite, None).await?;

            safely_abort(swap_id, db).await?;

            tracing::info!("Swap safely aborted");
        }
        Command::Redeem {
            swap_id,
            do_not_await_finality,
        } => {
            let db = open_db(config.data.dir.join("sqlite"), AccessMode::ReadWrite, None).await?;

            let bitcoin_wallet = init_bitcoin_wallet(&config, &seed, env_config).await?;

            let (txid, _) = redeem(
                swap_id,
                Arc::new(bitcoin_wallet),
                db,
                Finality::from_bool(do_not_await_finality),
            )
            .await?;

            tracing::info!("Redeem transaction successfully published with id {}", txid);
        }
        Command::ExportBitcoinWallet => {
            let bitcoin_wallet = init_bitcoin_wallet(&config, &seed, env_config).await?;
            let wallet_export = bitcoin_wallet.wallet_export("asb").await?;
            println!("{}", wallet_export.to_string())
        }
    }

    Ok(())
}

async fn init_bitcoin_wallet(
    config: &Config,
    seed: &Seed,
    env_config: swap::env::Config,
) -> Result<bitcoin::Wallet> {
    tracing::debug!("Opening Bitcoin wallet");
    let data_dir = &config.data.dir;
    let wallet = bitcoin::Wallet::new(
        config.bitcoin.electrum_rpc_url.clone(),
        data_dir,
        seed.derive_extended_private_key(env_config.bitcoin_network)?,
        env_config,
        config.bitcoin.target_block,
    )
    .await
    .context("Failed to initialize Bitcoin wallet")?;

    wallet.sync().await?;

    Ok(wallet)
}

async fn init_monero_wallet(
    config: &Config,
    env_config: swap::env::Config,
) -> Result<monero::Wallet> {
    tracing::debug!("Opening Monero wallet");
    let wallet = monero::Wallet::open_or_create(
        config.monero.wallet_rpc_url.clone(),
        DEFAULT_WALLET_NAME.to_string(),
        env_config,
    )
    .await?;

    Ok(wallet)
}
