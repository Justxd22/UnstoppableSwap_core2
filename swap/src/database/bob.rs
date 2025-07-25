use crate::monero::BlockHeight;
use crate::monero::TransferProof;
use crate::protocol::bob;
use crate::protocol::bob::BobState;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub enum Bob {
    Started {
        #[serde(with = "::bitcoin::amount::serde::as_sat")]
        btc_amount: bitcoin::Amount,
        #[serde(with = "crate::bitcoin::address_serde")]
        change_address: bitcoin::Address,
        tx_lock_fee: bitcoin::Amount,
    },
    ExecutionSetupDone {
        state2: bob::State2,
    },
    BtcLocked {
        state3: bob::State3,
        monero_wallet_restore_blockheight: BlockHeight,
    },
    XmrLockProofReceived {
        state: bob::State3,
        lock_transfer_proof: TransferProof,
        monero_wallet_restore_blockheight: BlockHeight,
    },
    XmrLocked {
        state4: bob::State4,
    },
    EncSigSent {
        state4: bob::State4,
    },
    BtcPunished {
        state: bob::State6,
        tx_lock_id: bitcoin::Txid,
    },
    BtcRedeemed(bob::State5),
    CancelTimelockExpired(bob::State6),
    BtcCancelled(bob::State6),
    BtcRefundPublished(bob::State6),
    BtcEarlyRefundPublished(bob::State6),
    Done(BobEndState),
}

#[derive(Clone, strum::Display, Debug, Deserialize, Serialize, PartialEq)]
pub enum BobEndState {
    SafelyAborted,
    XmrRedeemed { tx_lock_id: bitcoin::Txid },
    BtcRefunded(Box<bob::State6>),
    BtcEarlyRefunded(Box<bob::State6>),
}

impl From<BobState> for Bob {
    fn from(bob_state: BobState) -> Self {
        match bob_state {
            BobState::Started {
                btc_amount,
                change_address,
                tx_lock_fee,
            } => Bob::Started {
                btc_amount,
                change_address,
                tx_lock_fee,
            },
            BobState::SwapSetupCompleted(state2) => Bob::ExecutionSetupDone { state2 },
            BobState::BtcLocked {
                state3,
                monero_wallet_restore_blockheight,
            } => Bob::BtcLocked {
                state3,
                monero_wallet_restore_blockheight,
            },
            BobState::XmrLockProofReceived {
                state,
                lock_transfer_proof,
                monero_wallet_restore_blockheight,
            } => Bob::XmrLockProofReceived {
                state,
                lock_transfer_proof,
                monero_wallet_restore_blockheight,
            },
            BobState::XmrLocked(state4) => Bob::XmrLocked { state4 },
            BobState::EncSigSent(state4) => Bob::EncSigSent { state4 },
            BobState::BtcRedeemed(state5) => Bob::BtcRedeemed(state5),
            BobState::CancelTimelockExpired(state6) => Bob::CancelTimelockExpired(state6),
            BobState::BtcCancelled(state6) => Bob::BtcCancelled(state6),
            BobState::BtcRefundPublished(state6) => Bob::BtcRefundPublished(state6),
            BobState::BtcEarlyRefundPublished(state6) => Bob::BtcEarlyRefundPublished(state6),
            BobState::BtcPunished { state, tx_lock_id } => Bob::BtcPunished { state, tx_lock_id },
            BobState::BtcRefunded(state6) => Bob::Done(BobEndState::BtcRefunded(Box::new(state6))),
            BobState::XmrRedeemed { tx_lock_id } => {
                Bob::Done(BobEndState::XmrRedeemed { tx_lock_id })
            }
            BobState::BtcEarlyRefunded(state6) => {
                Bob::Done(BobEndState::BtcEarlyRefunded(Box::new(state6)))
            }
            BobState::SafelyAborted => Bob::Done(BobEndState::SafelyAborted),
        }
    }
}

impl From<Bob> for BobState {
    fn from(db_state: Bob) -> Self {
        match db_state {
            Bob::Started {
                btc_amount,
                change_address,
                tx_lock_fee,
            } => BobState::Started {
                btc_amount,
                change_address,
                tx_lock_fee,
            },
            Bob::ExecutionSetupDone { state2 } => BobState::SwapSetupCompleted(state2),
            Bob::BtcLocked {
                state3,
                monero_wallet_restore_blockheight,
            } => BobState::BtcLocked {
                state3,
                monero_wallet_restore_blockheight,
            },
            Bob::XmrLockProofReceived {
                state,
                lock_transfer_proof,
                monero_wallet_restore_blockheight,
            } => BobState::XmrLockProofReceived {
                state,
                lock_transfer_proof,
                monero_wallet_restore_blockheight,
            },
            Bob::XmrLocked { state4 } => BobState::XmrLocked(state4),
            Bob::EncSigSent { state4 } => BobState::EncSigSent(state4),
            Bob::BtcRedeemed(state5) => BobState::BtcRedeemed(state5),
            Bob::CancelTimelockExpired(state6) => BobState::CancelTimelockExpired(state6),
            Bob::BtcCancelled(state6) => BobState::BtcCancelled(state6),
            Bob::BtcRefundPublished(state6) => BobState::BtcRefundPublished(state6),
            Bob::BtcEarlyRefundPublished(state6) => BobState::BtcEarlyRefundPublished(state6),
            Bob::BtcPunished { state, tx_lock_id } => BobState::BtcPunished { state, tx_lock_id },
            Bob::Done(end_state) => match end_state {
                BobEndState::SafelyAborted => BobState::SafelyAborted,
                BobEndState::XmrRedeemed { tx_lock_id } => BobState::XmrRedeemed { tx_lock_id },
                BobEndState::BtcRefunded(state6) => BobState::BtcRefunded(*state6),
                BobEndState::BtcEarlyRefunded(state6) => BobState::BtcEarlyRefunded(*state6),
            },
        }
    }
}

impl fmt::Display for Bob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Bob::Started { .. } => write!(f, "Started"),
            Bob::ExecutionSetupDone { .. } => f.write_str("Execution setup done"),
            Bob::BtcLocked { .. } => f.write_str("Bitcoin locked"),
            Bob::XmrLockProofReceived { .. } => {
                f.write_str("XMR lock transaction transfer proof received")
            }
            Bob::XmrLocked { .. } => f.write_str("Monero locked"),
            Bob::CancelTimelockExpired(_) => f.write_str("Cancel timelock is expired"),
            Bob::BtcCancelled(_) => f.write_str("Bitcoin refundable"),
            Bob::BtcRefundPublished { .. } => f.write_str("Bitcoin refund published"),
            Bob::BtcEarlyRefundPublished { .. } => f.write_str("Bitcoin early refund published"),
            Bob::BtcRedeemed(_) => f.write_str("Monero redeemable"),
            Bob::Done(end_state) => write!(f, "Done: {}", end_state),
            Bob::EncSigSent { .. } => f.write_str("Encrypted signature sent"),
            Bob::BtcPunished { .. } => f.write_str("Bitcoin punished"),
        }
    }
}
