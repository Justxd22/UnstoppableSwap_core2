use crate::{asb, cli};
use libp2p::request_response::{self, ProtocolSupport};
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const PROTOCOL: &str = "/comit/xmr/btc/encrypted_signature/1.0.0";
type OutEvent = request_response::Event<Request, ()>;
type Message = request_response::Message<Request, ()>;

pub type Behaviour = request_response::cbor::Behaviour<Request, ()>;

#[derive(Debug, Clone, Copy, Default)]
pub struct EncryptedSignatureProtocol;

impl AsRef<str> for EncryptedSignatureProtocol {
    fn as_ref(&self) -> &str {
        PROTOCOL.as_bytes()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub swap_id: Uuid,
    pub tx_redeem_encsig: crate::bitcoin::EncryptedSignature,
}

pub fn alice() -> Behaviour {
    Behaviour::new(
        vec![(EncryptedSignatureProtocol, ProtocolSupport::Inbound)],
        request_response::Config::default(),
    )
}

pub fn bob() -> Behaviour {
    Behaviour::new(
        vec![(
            EncryptedSignatureProtocol,
            request_response::ProtocolSupport::Outbound,
        )],
        request_response::Config::default(),
    )
}

impl From<(PeerId, Message)> for asb::OutEvent {
    fn from((peer, message): (PeerId, Message)) -> Self {
        match message {
            Message::Request {
                request, channel, ..
            } => Self::EncryptedSignatureReceived {
                msg: request,
                channel,
                peer,
            },
            Message::Response { .. } => Self::unexpected_response(peer),
        }
    }
}
crate::impl_from_rr_event!(OutEvent, asb::OutEvent, PROTOCOL);

impl From<(PeerId, Message)> for cli::OutEvent {
    fn from((peer, message): (PeerId, Message)) -> Self {
        match message {
            Message::Request { .. } => Self::unexpected_request(peer),
            Message::Response { request_id, .. } => {
                Self::EncryptedSignatureAcknowledged { id: request_id }
            }
        }
    }
}
crate::impl_from_rr_event!(OutEvent, cli::OutEvent, PROTOCOL);
