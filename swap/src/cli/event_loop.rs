use crate::bitcoin::EncryptedSignature;
use crate::cli::behaviour::{Behaviour, OutEvent};
use crate::monero;
use crate::network::cooperative_xmr_redeem_after_punish::{self, Request, Response};
use crate::network::encrypted_signature;
use crate::network::quote::BidQuote;
use crate::network::swap_setup::bob::NewSwap;
use crate::protocol::bob::swap::has_already_processed_transfer_proof;
use crate::protocol::bob::{BobState, State2};
use crate::protocol::Database;
use anyhow::{Context, Result};
use futures::future::{BoxFuture, OptionFuture};
use futures::{FutureExt, StreamExt};
use libp2p::request_response::{OutboundFailure, OutboundRequestId, ResponseChannel};
use libp2p::swarm::dial_opts::DialOpts;
use libp2p::swarm::SwarmEvent;
use libp2p::{PeerId, Swarm};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

#[allow(missing_debug_implementations)]
pub struct EventLoop {
    swap_id: Uuid,
    swarm: libp2p::Swarm<Behaviour>,
    alice_peer_id: PeerId,
    db: Arc<dyn Database + Send + Sync>,

    // These streams represents outgoing requests that we have to make
    // These are essentially queues of requests that we will send to Alice once we are connected to her.
    quote_requests: bmrng::RequestReceiverStream<(), Result<BidQuote, OutboundFailure>>,
    cooperative_xmr_redeem_requests: bmrng::RequestReceiverStream<
        Uuid,
        Result<cooperative_xmr_redeem_after_punish::Response, OutboundFailure>,
    >,
    encrypted_signatures:
        bmrng::RequestReceiverStream<EncryptedSignature, Result<(), OutboundFailure>>,
    swap_setup_requests: bmrng::RequestReceiverStream<NewSwap, Result<State2>>,

    // These represents requests that are currently in-flight.
    // Meaning that we have sent them to Alice, but we have not yet received a response.
    // Once we get a response to a matching [`RequestId`], we will use the responder to relay the
    // response.
    inflight_quote_requests:
        HashMap<OutboundRequestId, bmrng::Responder<Result<BidQuote, OutboundFailure>>>,
    inflight_encrypted_signature_requests:
        HashMap<OutboundRequestId, bmrng::Responder<Result<(), OutboundFailure>>>,
    inflight_swap_setup: Option<bmrng::Responder<Result<State2>>>,
    inflight_cooperative_xmr_redeem_requests: HashMap<
        OutboundRequestId,
        bmrng::Responder<Result<cooperative_xmr_redeem_after_punish::Response, OutboundFailure>>,
    >,
    /// The sender we will use to relay incoming transfer proofs.
    transfer_proof: bmrng::RequestSender<monero::TransferProof, ()>,

    /// The future representing the successful handling of an incoming transfer
    /// proof.
    ///
    /// Once we've sent a transfer proof to the ongoing swap, this future waits
    /// until the swap took it "out" of the `EventLoopHandle`. As this future
    /// resolves, we use the `ResponseChannel` returned from it to send an ACK
    /// to Alice that we have successfully processed the transfer proof.
    pending_transfer_proof: OptionFuture<BoxFuture<'static, ResponseChannel<()>>>,
}

impl EventLoop {
    pub fn new(
        swap_id: Uuid,
        swarm: Swarm<Behaviour>,
        alice_peer_id: PeerId,
        db: Arc<dyn Database + Send + Sync>,
    ) -> Result<(Self, EventLoopHandle)> {
        let execution_setup = bmrng::channel_with_timeout(1, Duration::from_secs(60));
        let transfer_proof = bmrng::channel(1); // TODO(Libp2p Migration): Is it okay to have a channel without a timeout here?
        let encrypted_signature = bmrng::channel(1);
        let quote = bmrng::channel(1); // TODO(Libp2p Migration): WHY DOES THIS STILL TIMEOUT?
        let cooperative_xmr_redeem = bmrng::channel(1);

        let event_loop = EventLoop {
            swap_id,
            swarm,
            alice_peer_id,
            swap_setup_requests: execution_setup.1.into(),
            transfer_proof: transfer_proof.0,
            encrypted_signatures: encrypted_signature.1.into(),
            cooperative_xmr_redeem_requests: cooperative_xmr_redeem.1.into(),
            quote_requests: quote.1.into(),
            inflight_quote_requests: HashMap::default(),
            inflight_swap_setup: None,
            inflight_encrypted_signature_requests: HashMap::default(),
            inflight_cooperative_xmr_redeem_requests: HashMap::default(),
            pending_transfer_proof: OptionFuture::from(None),
            db,
        };

        let handle = EventLoopHandle {
            swap_setup: execution_setup.0,
            transfer_proof: transfer_proof.1,
            encrypted_signature: encrypted_signature.0,
            cooperative_xmr_redeem: cooperative_xmr_redeem.0,
            quote: quote.0,
        };

        Ok((event_loop, handle))
    }

    pub async fn run(mut self) {
        match self.swarm.dial(DialOpts::from(self.alice_peer_id)) {
            Ok(()) => {}
            Err(e) => {
                tracing::error!("Failed to initiate dial to Alice: {}", e);
                return;
            }
        }

        loop {
            // Note: We are making very elaborate use of `select!` macro's feature here. Make sure to read the documentation thoroughly: https://docs.rs/tokio/1.4.0/tokio/macro.select.html
            tokio::select! {
                swarm_event = self.swarm.select_next_some() => {
                    match swarm_event {
                        SwarmEvent::Behaviour(OutEvent::QuoteReceived { id, response }) => {
                            if let Some(responder) = self.inflight_quote_requests.remove(&id) {
                                let _ = responder.respond(Ok(response));
                            }
                        }
                        SwarmEvent::Behaviour(OutEvent::SwapSetupCompleted(response)) => {
                            if let Some(responder) = self.inflight_swap_setup.take() {
                                let _ = responder.respond(*response);
                            }
                        }
                        SwarmEvent::Behaviour(OutEvent::TransferProofReceived { msg, channel, peer }) => {
                            let swap_id = msg.swap_id;

                            if swap_id == self.swap_id {
                                if peer != self.alice_peer_id {
                                    tracing::warn!(
                                                %swap_id,
                                                "Ignoring malicious transfer proof from {}, expected to receive it from {}",
                                                peer,
                                                self.alice_peer_id);
                                            continue;
                                }

                                // Immediately acknowledge if we've already processed this transfer proof
                                // This handles the case where Alice didn't receive our previous acknowledgment
                                // and is retrying sending the transfer proof
                                if let Ok(state) = self.db.get_state(swap_id).await {
                                    let state: BobState = state.try_into()
                                        .expect("Bobs database only contains Bob states");

                                    if has_already_processed_transfer_proof(&state) {
                                        tracing::warn!("Received transfer proof for swap {} but we are already in state {}. Acknowledging immediately. Alice most likely did not receive the acknowledgment when we sent it before", swap_id, state);

                                        // We set this to a future that will resolve immediately, and returns the channel
                                        // This will be resolved in the next iteration of the event loop, and a response will be sent to Alice
                                        self.pending_transfer_proof = OptionFuture::from(Some(async move {
                                            channel
                                        }.boxed()));

                                        continue;
                                    }
                                }

                                let mut responder = match self.transfer_proof.send(msg.tx_lock_proof).await {
                                    Ok(responder) => responder,
                                    Err(e) => {
                                        tracing::warn!("Failed to pass on transfer proof: {:#}", e);
                                        continue;
                                    }
                                };

                                self.pending_transfer_proof = OptionFuture::from(Some(async move {
                                    let _ = responder.recv().await;

                                    channel
                                }.boxed()));
                            }else {
                                // Check if the transfer proof is sent from the correct peer and if we have a record of the swap
                                match self.db.get_peer_id(swap_id).await {
                                    // We have a record of the swap
                                    Ok(buffer_swap_alice_peer_id) => {
                                        if buffer_swap_alice_peer_id == self.alice_peer_id {
                                            // Save transfer proof in the database such that we can process it later when we resume the swap
                                            match self.db.insert_buffered_transfer_proof(swap_id, msg.tx_lock_proof).await {
                                                Ok(_) => {
                                                    tracing::info!("Received transfer proof for swap {} while running swap {}. Buffering this transfer proof in the database for later retrieval", swap_id, self.swap_id);
                                                    let _ = self.swarm.behaviour_mut().transfer_proof.send_response(channel, ());
                                                }
                                                Err(e) => {
                                                    tracing::error!("Failed to buffer transfer proof for swap {}: {:#}", swap_id, e);
                                                }
                                            };
                                        }else {
                                            tracing::warn!(
                                                %swap_id,
                                                "Ignoring malicious transfer proof from {}, expected to receive it from {}",
                                                self.swap_id,
                                                buffer_swap_alice_peer_id);
                                        }
                                    },
                                    // We do not have a record of the swap or an error occurred while retrieving the peer id of Alice
                                    Err(e) => {
                                        if let Some(sqlx::Error::RowNotFound) = e.downcast_ref::<sqlx::Error>() {
                                            tracing::warn!("Ignoring transfer proof for swap {} while running swap {}. We do not have a record of this swap", swap_id, self.swap_id);
                                        } else {
                                            tracing::error!("Ignoring transfer proof for swap {} while running swap {}. Failed to retrieve the peer id of Alice for the corresponding swap: {:#}", swap_id, self.swap_id, e);
                                        }
                                    }
                                }
                            }
                        }
                        SwarmEvent::Behaviour(OutEvent::EncryptedSignatureAcknowledged { id }) => {
                            if let Some(responder) = self.inflight_encrypted_signature_requests.remove(&id) {
                                let _ = responder.respond(Ok(()));
                            }
                        }
                        SwarmEvent::Behaviour(OutEvent::CooperativeXmrRedeemFulfilled { id, swap_id, s_a }) => {
                            if let Some(responder) = self.inflight_cooperative_xmr_redeem_requests.remove(&id) {
                                let _ = responder.respond(Ok(Response::Fullfilled { s_a, swap_id }));
                            }
                        }
                        SwarmEvent::Behaviour(OutEvent::CooperativeXmrRedeemRejected { id, swap_id, reason }) => {
                            if let Some(responder) = self.inflight_cooperative_xmr_redeem_requests.remove(&id) {
                                let _ = responder.respond(Ok(Response::Rejected { reason, swap_id }));
                            }
                        }
                        SwarmEvent::Behaviour(OutEvent::Failure { peer, error }) => {
                            tracing::warn!(%peer, err = %error, "Communication error");
                            return;
                        }
                        SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } if peer_id == self.alice_peer_id => {
                            tracing::info!(peer_id = %endpoint.get_remote_address(), "Connected to Alice");
                        }
                        SwarmEvent::Dialing { peer_id: Some(alice_peer_id), connection_id } if alice_peer_id == self.alice_peer_id => {
                            tracing::debug!(%alice_peer_id, %connection_id, "Dialing Alice");
                        }
                        SwarmEvent::ConnectionClosed { peer_id, endpoint, num_established, cause: Some(error), connection_id } if peer_id == self.alice_peer_id && num_established == 0 => {
                            tracing::warn!(peer_id = %endpoint.get_remote_address(), cause = %error, %connection_id, "Lost connection to Alice");
                        }
                        SwarmEvent::ConnectionClosed { peer_id, num_established, cause: None, .. } if peer_id == self.alice_peer_id && num_established == 0 => {
                            // no error means the disconnection was requested
                            tracing::info!("Successfully closed connection to Alice");
                            return;
                        }
                        SwarmEvent::OutgoingConnectionError { peer_id: Some(alice_peer_id),  error, connection_id } if alice_peer_id == self.alice_peer_id => {
                            tracing::warn!(%alice_peer_id, %connection_id, %error, "Failed to connect to Alice");

                            if let Some(duration) = self.swarm.behaviour_mut().redial.until_next_redial() {
                                tracing::info!(seconds_until_next_redial = %duration.as_secs(), "Waiting for next redial attempt");
                            }
                        }
                        SwarmEvent::Behaviour(OutEvent::OutboundRequestResponseFailure {peer, error, request_id, protocol}) => {
                            tracing::error!(
                                %peer,
                                %request_id,
                                %error,
                                %protocol,
                                "Failed to send request-response request to peer");

                            // If we fail to send a request-response request, we should notify the responder that the request failed
                            // We will remove the responder from the inflight requests and respond with an error

                            // Check for encrypted signature requests
                            if let Some(responder) = self.inflight_encrypted_signature_requests.remove(&request_id) {
                                let _ = responder.respond(Err(error));
                                continue;
                            }

                            // Check for quote requests
                            if let Some(responder) = self.inflight_quote_requests.remove(&request_id) {
                                let _ = responder.respond(Err(error));
                                continue;
                            }

                            // Check for cooperative xmr redeem requests
                            if let Some(responder) = self.inflight_cooperative_xmr_redeem_requests.remove(&request_id) {
                                let _ = responder.respond(Err(error));
                                continue;
                            }
                        }
                        SwarmEvent::Behaviour(OutEvent::InboundRequestResponseFailure {peer, error, request_id, protocol}) => {
                            tracing::error!(
                                %peer,
                                %request_id,
                                %error,
                                %protocol,
                                "Failed to receive request-response request from peer");
                        }
                        _ => {}
                    }
                },

                // Handle to-be-sent requests for all our network protocols.
                // Use `self.is_connected_to_alice` as a guard to "buffer" requests until we are connected.
                Some(((), responder)) = self.quote_requests.next().fuse(), if self.is_connected_to_alice() => {
                    let id = self.swarm.behaviour_mut().quote.send_request(&self.alice_peer_id, ());
                    self.inflight_quote_requests.insert(id, responder);
                },
                Some((swap, responder)) = self.swap_setup_requests.next().fuse(), if self.is_connected_to_alice() => {
                    self.swarm.behaviour_mut().swap_setup.start(self.alice_peer_id, swap).await;
                    self.inflight_swap_setup = Some(responder);
                },
                Some((tx_redeem_encsig, responder)) = self.encrypted_signatures.next().fuse(), if self.is_connected_to_alice() => {
                    let request = encrypted_signature::Request {
                        swap_id: self.swap_id,
                        tx_redeem_encsig
                    };

                    let id = self.swarm.behaviour_mut().encrypted_signature.send_request(&self.alice_peer_id, request);
                    self.inflight_encrypted_signature_requests.insert(id, responder);
                },

                Some(response_channel) = &mut self.pending_transfer_proof => {
                    if let Err(_) = self.swarm.behaviour_mut().transfer_proof.send_response(response_channel, ()) {
                        tracing::warn!("Failed to send acknowledgment to Alice that we have received the transfer proof");
                    } else {
                        self.pending_transfer_proof = OptionFuture::from(None);
                    }
                },

                Some((swap_id, responder)) = self.cooperative_xmr_redeem_requests.next().fuse(), if self.is_connected_to_alice() => {
                    let id = self.swarm.behaviour_mut().cooperative_xmr_redeem.send_request(&self.alice_peer_id, Request {
                        swap_id
                    });
                    self.inflight_cooperative_xmr_redeem_requests.insert(id, responder);
                },
            }
        }
    }

    fn is_connected_to_alice(&self) -> bool {
        self.swarm.is_connected(&self.alice_peer_id)
    }
}

#[derive(Debug)]
pub struct EventLoopHandle {
    swap_setup: bmrng::RequestSender<NewSwap, Result<State2>>,
    transfer_proof: bmrng::RequestReceiver<monero::TransferProof, ()>,
    encrypted_signature: bmrng::RequestSender<EncryptedSignature, Result<(), OutboundFailure>>,
    quote: bmrng::RequestSender<(), Result<BidQuote, OutboundFailure>>,
    cooperative_xmr_redeem: bmrng::RequestSender<
        Uuid,
        Result<cooperative_xmr_redeem_after_punish::Response, OutboundFailure>,
    >,
}

impl EventLoopHandle {
    pub async fn setup_swap(&mut self, swap: NewSwap) -> Result<State2> {
        self.swap_setup.send_receive(swap).await?
    }

    pub async fn recv_transfer_proof(&mut self) -> Result<monero::TransferProof> {
        let (transfer_proof, responder) = self
            .transfer_proof
            .recv()
            .await
            .context("Failed to receive transfer proof")?;

        responder
            .respond(())
            .context("Failed to acknowledge receipt of transfer proof")?;

        Ok(transfer_proof)
    }

    pub async fn request_quote(&mut self) -> Result<BidQuote> {
        tracing::debug!("Requesting quote");
        self.quote
            .send_receive(())
            .await
            .context("Failed to receive quote through event loop channel")?
            .context("Failed to request quote due to a network error")
    }

    pub async fn request_cooperative_xmr_redeem(&mut self, swap_id: Uuid) -> Result<Response> {
        self.cooperative_xmr_redeem
            .send_receive(swap_id)
            .await
            .context("Failed to request cooperative XMR redeem through event loop channel")?
            .context("Failed to request cooperative XMR redeem due to a network error")
    }

    pub async fn send_encrypted_signature(
        &mut self,
        tx_redeem_encsig: EncryptedSignature,
    ) -> Result<()> {
        // We will retry indefinitely until we succeed
        let backoff = backoff::ExponentialBackoffBuilder::new()
            .with_max_elapsed_time(None)
            .with_max_interval(Duration::from_secs(60))
            .build();

        backoff::future::retry(backoff, || async {
            match self.encrypted_signature.send_receive(tx_redeem_encsig.clone()).await {
                    Ok(Ok(_)) => Ok(()),
                    Ok(Err(err)) => {
                        tracing::warn!(%err, "Failed to send encrypted signature due to a network error. Will retry");
                        Err(backoff::Error::transient(anyhow::anyhow!(err)))
                    }
                    Err(bmrng::error::RequestError::RecvTimeoutError) => {
                        unreachable!("We construct the channel without a timeout, so this should never happen")
                    }
                    Err(err) => {
                        // The MSCP channel has failed. We do not retry this because this error means that either the channel was closed or the receiver has been dropped.
                        // Both of these cases are permanent and we should not retry.
                        // TODO(Libp2p Migration): Is this correct?
                        tracing::error!(%err, "Failed to communicate transfer proof through event loop channel. We will not retry.");
                        Err(backoff::Error::permanent(anyhow::anyhow!(err).context("Failed to communicate transfer proof through event loop channel")))
                    }
                }
            })
            .await
    }
}
