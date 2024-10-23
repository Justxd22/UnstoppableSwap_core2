use crate::network::swap_setup::{protocol, BlockchainNetwork, SpotPriceError, SpotPriceResponse};
use crate::protocol::bob::{State0, State2};
use crate::protocol::{Message1, Message3};
use crate::{bitcoin, cli, env, monero};
use anyhow::Result;
use futures::future::{BoxFuture, OptionFuture};
use futures::FutureExt;
use libp2p::core::upgrade;
use libp2p::swarm::{
    ConnectionDenied, ConnectionHandler, ConnectionHandlerEvent, ConnectionId, FromSwarm,
    NetworkBehaviour, SubstreamProtocol, THandler, THandlerInEvent, THandlerOutEvent, ToSwarm,
};
use libp2p::{Multiaddr, PeerId};
use std::collections::VecDeque;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use uuid::Uuid;
use futures::AsyncWriteExt;

use super::{read_cbor_message, write_cbor_message, SpotPriceRequest};

#[allow(missing_debug_implementations)]
pub struct Behaviour {
    env_config: env::Config,
    bitcoin_wallet: Arc<bitcoin::Wallet>,
    new_swaps: VecDeque<(PeerId, NewSwap)>,
    completed_swaps: VecDeque<(PeerId, Completed)>,
}

impl Behaviour {
    pub fn new(env_config: env::Config, bitcoin_wallet: Arc<bitcoin::Wallet>) -> Self {
        Self {
            env_config,
            bitcoin_wallet,
            new_swaps: VecDeque::default(),
            completed_swaps: VecDeque::default(),
        }
    }

    pub async fn start(&mut self, alice: PeerId, swap: NewSwap) {
        self.new_swaps.push_back((alice, swap))
    }
}

impl From<Completed> for cli::OutEvent {
    fn from(completed: Completed) -> Self {
        cli::OutEvent::SwapSetupCompleted(Box::new(completed.0))
    }
}

impl NetworkBehaviour for Behaviour {
    type ConnectionHandler = Handler;
    type ToSwarm = Completed;

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(Handler::new(self.env_config, self.bitcoin_wallet.clone()))
    }

    fn handle_established_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        addr: &Multiaddr,
        role_override: libp2p::core::Endpoint,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(Handler::new(self.env_config, self.bitcoin_wallet.clone()))
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        match event {
            FromSwarm::ConnectionEstablished(_) => {}
            FromSwarm::ConnectionClosed(_) => {}
            _ => {}
        }
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        _connection_id: libp2p::swarm::ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        self.completed_swaps.push_back((peer_id, event));
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        if let Some((peer, completed)) = self.completed_swaps.pop_front() {
            return Poll::Ready(ToSwarm::GenerateEvent(completed));
        }
        Poll::Pending
    }
}

type OutboundStream = BoxFuture<'static, Result<State2, Error>>;

pub struct Handler {
    outbound_stream: OptionFuture<OutboundStream>,
    env_config: env::Config,
    timeout: Duration,
    new_swaps: VecDeque<NewSwap>,
    bitcoin_wallet: Arc<bitcoin::Wallet>,
    keep_alive: bool,
}

impl Handler {
    fn new(env_config: env::Config, bitcoin_wallet: Arc<bitcoin::Wallet>) -> Self {
        Self {
            env_config,
            outbound_stream: OptionFuture::from(None),
            timeout: Duration::from_secs(120),
            new_swaps: VecDeque::default(),
            bitcoin_wallet,
            keep_alive: true,
        }
    }
}

#[derive(Debug)]
pub struct NewSwap {
    pub swap_id: Uuid,
    pub btc: bitcoin::Amount,
    pub tx_refund_fee: bitcoin::Amount,
    pub tx_cancel_fee: bitcoin::Amount,
    pub bitcoin_refund_address: bitcoin::Address,
}

#[derive(Debug)]
pub struct Completed(Result<State2>);

impl ConnectionHandler for Handler {
    type FromBehaviour = NewSwap;
    type ToBehaviour = Completed;
    type InboundProtocol = upgrade::DeniedUpgrade;
    type OutboundProtocol = protocol::SwapSetup;
    type InboundOpenInfo = ();
    type OutboundOpenInfo = NewSwap;

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundProtocol, Self::InboundOpenInfo> {
        SubstreamProtocol::new(upgrade::DeniedUpgrade, ())
    }

    fn on_behaviour_event(&mut self, new_swap: Self::FromBehaviour) {
        self.new_swaps.push_back(new_swap);
    }

    fn connection_keep_alive(&self) -> bool {
        self.keep_alive
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<
        ConnectionHandlerEvent<Self::OutboundProtocol, Self::OutboundOpenInfo, Self::ToBehaviour>,
    > {
        if let Some(new_swap) = self.new_swaps.pop_front() {
            self.keep_alive = true;
            return Poll::Ready(ConnectionHandlerEvent::OutboundSubstreamRequest {
                protocol: SubstreamProtocol::new(protocol::new(), new_swap),
            });
        }

        if let Poll::Ready(Some(result)) = self.outbound_stream.poll_unpin(cx) {
            self.outbound_stream = None.into();
            self.keep_alive = false;
            return Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(Completed(result.map_err(anyhow::Error::from))));
        }

        Poll::Pending
    }

    fn on_connection_event(
        &mut self,
        event: libp2p::swarm::handler::ConnectionEvent<
            Self::InboundProtocol,
            Self::OutboundProtocol,
            Self::InboundOpenInfo,
            Self::OutboundOpenInfo,
        >,
    ) {
        match event {
            libp2p::swarm::handler::ConnectionEvent::FullyNegotiatedInbound(_) => {
                unreachable!("Bob does not support inbound substreams")
            }
            libp2p::swarm::handler::ConnectionEvent::FullyNegotiatedOutbound(outbound) => {
                let mut substream = outbound.protocol;
                let info = outbound.info;

                let bitcoin_wallet = self.bitcoin_wallet.clone();
                let env_config = self.env_config;

                let bitcoin_wallet = self.bitcoin_wallet.clone();
                let env_config = self.env_config;

                let protocol = tokio::time::timeout(self.timeout, async move {
                    write_cbor_message(
                        &mut substream,
                        SpotPriceRequest {
                            btc: info.btc,
                            blockchain_network: BlockchainNetwork {
                                bitcoin: env_config.bitcoin_network,
                                monero: env_config.monero_network,
                            },
                        },
                    )
                    .await?;

                    let xmr = Result::from(
                        read_cbor_message::<SpotPriceResponse>(&mut substream).await?,
                    )?;

                    let state0 = State0::new(
                        info.swap_id,
                        &mut rand::thread_rng(),
                        info.btc,
                        xmr,
                        env_config.bitcoin_cancel_timelock,
                        env_config.bitcoin_punish_timelock,
                        info.bitcoin_refund_address,
                        env_config.monero_finality_confirmations,
                        info.tx_refund_fee,
                        info.tx_cancel_fee,
                    );

                    write_cbor_message(&mut substream, state0.next_message()).await?;
                    let message1 = read_cbor_message::<Message1>(&mut substream).await?;
                    let state1 = state0.receive(bitcoin_wallet.as_ref(), message1).await?;

                    write_cbor_message(&mut substream, state1.next_message()).await?;
                    let message3 = read_cbor_message::<Message3>(&mut substream).await?;
                    let state2 = state1.receive(message3)?;

                    write_cbor_message(&mut substream, state2.next_message()).await?;

                    substream.flush().await?;
                    substream.close().await?;

                    Ok(state2)
                });

                let max_seconds = self.timeout.as_secs();
                self.outbound_stream = OptionFuture::from(Some(Box::pin(async move {
                    protocol.await.map_err(|e| match e {
                        tokio::time::error::Elapsed { .. } => Error::Timeout {
                            seconds: max_seconds,
                        },
                        _ => Error::Other,
                    })?
                }) as OutboundStream));
                self.keep_alive = true; // Ensure the connection stays alive while processing
            }
            libp2p::swarm::handler::ConnectionEvent::DialUpgradeError(dial_upgrade_err) => {
                // Handle dial upgrade error if needed
                self.keep_alive = false; // Consider setting to false on error
            }
            _ => {}
        }
    }
}

impl From<SpotPriceResponse> for Result<monero::Amount, Error> {
    fn from(response: SpotPriceResponse) -> Self {
        match response {
            SpotPriceResponse::Xmr(amount) => Ok(amount),
            SpotPriceResponse::Error(e) => Err(e.into()),
        }
    }
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("Seller currently does not accept incoming swap requests, please try again later")]
    NoSwapsAccepted,
    #[error("Seller refused to buy {buy} because the minimum configured buy limit is {min}")]
    AmountBelowMinimum {
        min: bitcoin::Amount,
        buy: bitcoin::Amount,
    },
    #[error("Seller refused to buy {buy} because the maximum configured buy limit is {max}")]
    AmountAboveMaximum {
        max: bitcoin::Amount,
        buy: bitcoin::Amount,
    },
    #[error("Seller's XMR balance is currently too low to fulfill the swap request to buy {buy}, please try again later")]
    BalanceTooLow { buy: bitcoin::Amount },

    #[error("Seller blockchain network {asb:?} setup did not match your blockchain network setup {cli:?}")]
    BlockchainNetworkMismatch {
        cli: BlockchainNetwork,
        asb: BlockchainNetwork,
    },

    #[error("Failed to complete swap setup within {seconds}s")]
    Timeout { seconds: u64 },

    /// To be used for errors that cannot be explained on the CLI side (e.g.
    /// rate update problems on the seller side)
    #[error("Seller encountered a problem, please try again later.")]
    Other,
}

impl From<SpotPriceError> for Error {
    fn from(error: SpotPriceError) -> Self {
        match error {
            SpotPriceError::NoSwapsAccepted => Error::NoSwapsAccepted,
            SpotPriceError::AmountBelowMinimum { min, buy } => {
                Error::AmountBelowMinimum { min, buy }
            }
            SpotPriceError::AmountAboveMaximum { max, buy } => {
                Error::AmountAboveMaximum { max, buy }
            }
            SpotPriceError::BalanceTooLow { buy } => Error::BalanceTooLow { buy },
            SpotPriceError::BlockchainNetworkMismatch { cli, asb } => {
                Error::BlockchainNetworkMismatch { cli, asb }
            }
            SpotPriceError::Other => Error::Other,
        }
    }
}

impl From<anyhow::Error> for Error {
    fn from(error: anyhow::Error) -> Self {
        // This is not good we are just swallowing the error here
        // TODO: Libp2p Upgrade: We should find a better way to convert these errors in the entire file here into each other
        // This doesnt seem optimal at all
        // Incredibly ugly code and we lose a lot of valueale information here
        Error::Other
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        // This is not good we are just swallowing the error here
        Error::Other
    }
}