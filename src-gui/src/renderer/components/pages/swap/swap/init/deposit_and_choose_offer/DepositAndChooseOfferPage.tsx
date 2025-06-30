import { Typography, Box, Paper, Divider } from "@mui/material";
import ActionableMonospaceTextBox from "renderer/components/other/ActionableMonospaceTextBox";
import MakerOfferItem from "./MakerOfferItem";
import { usePendingSelectMakerApproval } from "store/hooks";
import MakerDiscoveryStatus from "./MakerDiscoveryStatus";
import { TauriSwapProgressEventContent } from "models/tauriModelExt";
import { SatsAmount } from "renderer/components/other/Units";
import _, { sortBy } from "lodash";

export default function DepositAndChooseOfferPage({
  deposit_address,
  max_giveable,
  known_quotes,
}: TauriSwapProgressEventContent<"WaitingForBtcDeposit">) {
  const pendingSelectMakerApprovals = usePendingSelectMakerApproval();

  const makerOffers = _.chain(
    sortBy(
      pendingSelectMakerApprovals,
      (approval) => -approval.content.expiration_ts,
    ),
  )
    .map((approval) => ({
      quoteWithAddress: approval.content.details.content.maker,
      requestId: approval.content.request_id,
    }))
    .concat(
      known_quotes.map((quote) => ({
        quoteWithAddress: quote,
        requestId: null,
      })),
    )
    .sortBy((quote) => quote.quoteWithAddress.quote.price)
    .sortBy((quote) => (quote.requestId ? 0 : 1))
    // .uniqBy((quote) => quote.quoteWithAddress.peer_id)
    .value();

  return (
    <>
      <Box
        sx={{
          display: "flex",
          flexDirection: "column",
          gap: 3,
        }}
      >
        <Paper
          elevation={8}
          sx={{
            padding: 2,
            display: "flex",
            flexDirection: { xs: "column", md: "row" },
            gap: 2,
          }}
        >
          <Box sx={{ flexGrow: 1, flexShrink: 0, minWidth: "12em" }}>
            <Typography variant="body1">Bitcoin Balance</Typography>
            <Typography variant="h5">
              <SatsAmount amount={max_giveable} />
            </Typography>
          </Box>

          <Divider
            orientation="vertical"
            flexItem
            sx={{
              marginX: { xs: 0, md: 1 },
              marginY: { xs: 1, md: 0 },
              display: { xs: "none", md: "block" },
            }}
          />
          <Divider
            orientation="horizontal"
            flexItem
            sx={{
              marginX: { xs: 0, md: 1 },
              marginY: { xs: 1, md: 0 },
              display: { xs: "block", md: "none" },
            }}
          />

          <Box
            sx={{
              flexShrink: 1,
              display: "flex",
              flexDirection: "column",
              gap: 1,
            }}
          >
            <Typography variant="body1">Deposit</Typography>
            <Typography variant="body2" color="text.secondary">
              Send Bitcoin to your internal wallet to swap your desired amount
              of Monero
            </Typography>
            <ActionableMonospaceTextBox content={deposit_address} />
          </Box>
        </Paper>

        {/* Available Makers Section */}
        <Box>
          <Box
            sx={{
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              mb: 2,
            }}
          >
            <Typography variant="h5">Select an offer</Typography>
          </Box>

          {/* Maker Discovery Status */}
          <MakerDiscoveryStatus />

          {/* Real Maker Offers */}
          <Box>
            {makerOffers.length > 0 && (
              <Box sx={{ display: "flex", flexDirection: "column", gap: 1 }}>
                {makerOffers.map((quote, index) => {
                  return (
                    <MakerOfferItem
                      key={index}
                      quoteWithAddress={quote.quoteWithAddress}
                      requestId={quote.requestId}
                    />
                  );
                })}
              </Box>
            )}

            {/* TODO: Differentiate between no makers found and still loading */}
            {makerOffers.length === 0 && (
              <Paper variant="outlined" sx={{ p: 3, textAlign: "center" }}>
                <Typography variant="body1" color="textSecondary">
                  Searching for available makers...
                </Typography>
                <Typography
                  variant="body2"
                  color="textSecondary"
                  sx={{ mt: 1 }}
                >
                  Please wait while we find the best offers for your swap.
                </Typography>
              </Paper>
            )}
          </Box>
        </Box>
      </Box>
    </>
  );
}
