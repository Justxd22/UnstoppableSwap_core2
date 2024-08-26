import { Box, DialogContentText } from "@mui/material";
import { TauriSwapProgressEventContent } from "models/tauriModelExt";
import MoneroTransactionInfoBox from "../../MoneroTransactionInfoBox";

export default function XmrLockTxInMempoolPage({
  xmr_lock_tx_confirmations,
  xmr_lock_txid,
}: TauriSwapProgressEventContent<"XmrLockTxInMempool">) {
  const additionalContent = `Confirmations: ${xmr_lock_tx_confirmations}/10`;

  return (
    <Box>
      <DialogContentText>
        They have published their Monero lock transaction. The swap will proceed
        once the transaction has been confirmed.
      </DialogContentText>

      <MoneroTransactionInfoBox
        title="Monero Lock Transaction"
        txId={xmr_lock_txid}
        additionalContent={additionalContent}
        loading
      />
    </Box>
  );
}
