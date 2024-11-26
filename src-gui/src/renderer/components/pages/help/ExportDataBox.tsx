import {
  Box,
  Typography,
  Dialog,
  DialogTitle,
  DialogContent,
  DialogActions,
  Button,
  Link,
  DialogContentText,
} from "@mui/material";
import makeStyles from '@mui/styles/makeStyles';
import InfoBox from "renderer/components/modal/swap/InfoBox";
import { useState } from "react";
import { getWalletDescriptor } from "renderer/rpc";
import { ExportBitcoinWalletResponse } from "models/tauriModel";
import PromiseInvokeButton from "renderer/components/PromiseInvokeButton";
import ActionableMonospaceTextBox from "renderer/components/other/ActionableMonospaceTextBox";

const useStyles = makeStyles((theme) => ({
  content: {
    display: "flex",
    flexDirection: "column",
    alignItems: "flex-start",
    gap: theme.spacing(2),
  }
}));

export default function ExportDataBox() {
  const classes = useStyles();
  const [walletDescriptor, setWalletDescriptor] = useState<ExportBitcoinWalletResponse | null>(null);

  const handleCloseDialog = () => {
    setWalletDescriptor(null);
  };

  return (
    <InfoBox
      title="Export Bitcoin Wallet"
      icon={null}
      loading={false}
      mainContent={
        <Box className={classes.content}>
          <Typography variant="subtitle2">
            You can export the wallet descriptor of the interal Bitcoin wallet for backup or recovery purposes. Please make sure to store it securely.
          </Typography>
        </Box>
      }
      additionalContent={
        <>
          <PromiseInvokeButton
            variant="outlined"
            onInvoke={getWalletDescriptor}
            onSuccess={setWalletDescriptor}
            displayErrorSnackbar={true}
          >
            Reveal Bitcoin Wallet Private Key
          </PromiseInvokeButton>
          {walletDescriptor !== null && (
            <WalletDescriptorModal
              open={walletDescriptor !== null}
              onClose={handleCloseDialog}
              walletDescriptor={walletDescriptor}
            />
          )}
        </>
      }
    />
  );
}

function WalletDescriptorModal({
  open,
  onClose,
  walletDescriptor,
}: {
  open: boolean;
  onClose: () => void;
  walletDescriptor: ExportBitcoinWalletResponse;
}) {
  const parsedDescriptor = JSON.parse(walletDescriptor.wallet_descriptor.descriptor);
  const stringifiedDescriptor = JSON.stringify(parsedDescriptor, null, 4);

  return (
    (<Dialog open={open} onClose={onClose} maxWidth="md" fullWidth>
      <DialogTitle>Bitcoin Wallet Descriptor</DialogTitle>
      <DialogContent>
        <DialogContentText>
          <ul style={{ marginTop: 0 }}>
            <li>
              The text below contains the wallet descriptor of the internal Bitcoin wallet. It contains your private key and can be used to derive your wallet. It should thus be stored securely.
            </li>
            <li>
              It can be imported into other Bitcoin wallets or services that support the descriptor format.
            </li>
            <li>
              For more information on what to do with the descriptor, see our 
              {" "}<Link
              href="https://github.com/UnstoppableSwap/core/blob/master/dev-docs/asb/README.md#exporting-the-bitcoin-wallet-descriptor"
              target="_blank"
              underline="hover">documentation</Link>
            </li>
          </ul>
        </DialogContentText>
        <ActionableMonospaceTextBox
          content={stringifiedDescriptor}
          displayCopyIcon={true}
          enableQrCode={false}
        />
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose} color="primary" variant="contained">
          Done
        </Button>
      </DialogActions>
    </Dialog>)
  );
}
