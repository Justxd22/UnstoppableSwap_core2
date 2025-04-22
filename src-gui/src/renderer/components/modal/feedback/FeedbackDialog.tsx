import {
  Box,
  Button,
  Checkbox,
  Dialog,
  DialogActions,
  DialogContent,
  DialogContentText,
  DialogTitle,
  FormControlLabel,
  IconButton,
  MenuItem,
  Paper,
  Select,
  TextField,
  Typography,
} from "@material-ui/core";
import { useSnackbar } from "notistack";
import { useEffect, useState } from "react";
import TruncatedText from "renderer/components/other/TruncatedText";
import { store } from "renderer/store/storeRenderer";
import { useActiveSwapInfo, useAppSelector } from "store/hooks";
import { logsToRawString, parseDateString } from "utils/parseUtils";
import { submitFeedbackViaHttp } from "../../../api";
import LoadingButton from "../../other/LoadingButton";
import { PiconeroAmount } from "../../other/Units";
import { getLogsOfSwap } from "renderer/rpc";
import logger from "utils/logger";
import { Visibility } from "@material-ui/icons";
import CliLogsBox from "renderer/components/other/RenderedCliLog";
import { CliLog, parseCliLogString } from "models/cliModel";

async function submitFeedback(body: string, swapId: string | null, swapLogs: string | null, daemonLogs: string | null) {
  let attachedBody = "";

  if (swapId !== null) {
    const swapInfo = store.getState().rpc.state.swapInfos[swapId];

    if (swapInfo === undefined) {
      throw new Error(`Swap with id ${swapId} not found`);
    }

    attachedBody = `${JSON.stringify(swapInfo, null, 4)}\n\nLogs: ${swapLogs ?? ""}`;
  }

  if (daemonLogs !== null) {
    attachedBody += `\n\nDaemon Logs: ${daemonLogs ?? ""}`;
  }

  console.log(`Sending feedback with attachement: \`\n${attachedBody}\``)
  await submitFeedbackViaHttp(body, attachedBody);
}

/*
 * This component is a dialog that allows the user to submit feedback to the
 * developers. The user can enter a message and optionally attach logs from a
 * specific swap.
 * selectedSwap = null means no swap is attached
 */
function SwapSelectDropDown({
  selectedSwap,
  setSelectedSwap,
}: {
  selectedSwap: string | null;
  setSelectedSwap: (swapId: string | null) => void;
}) {
  const swaps = useAppSelector((state) =>
    Object.values(state.rpc.state.swapInfos),
  );

  return (
    <Select
      value={selectedSwap ?? ""}
      variant="outlined"
      onChange={(e) => setSelectedSwap(e.target.value as string || null)}
      style={{ width: "100%" }}
      displayEmpty
    >
      <MenuItem value="">Do not attach a swap</MenuItem>
      {swaps.map((swap) => (
        <MenuItem value={swap.swap_id} key={swap.swap_id}>
          Swap{" "}<TruncatedText>{swap.swap_id}</TruncatedText>{" "}from{" "}
          {new Date(parseDateString(swap.start_date)).toDateString()} (
          <PiconeroAmount amount={swap.xmr_amount} />)
        </MenuItem>
      ))}
    </Select>
  );
}

const MAX_FEEDBACK_LENGTH = 4000;

export default function FeedbackDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const [pending, setPending] = useState(false);
  const [bodyText, setBodyText] = useState("");
  const currentSwapId = useActiveSwapInfo();

  const { enqueueSnackbar } = useSnackbar();

  const [selectedSwap, setSelectedSwap] = useState<
    string | null
  >(currentSwapId?.swap_id || null);
  const [swapLogs, setSwapLogs] = useState<(string | CliLog)[] | null>(null);
  const [attachDaemonLogs, setAttachDaemonLogs] = useState(true);

  const [daemonLogs, setDaemonLogs] = useState<(string | CliLog)[] | null>(null);

  useEffect(() => {
    // Reset logs if no swap is selected
    if (selectedSwap === null) {
      setSwapLogs(null);
      return;
    }

    // Fetch the logs from the rust backend and update the state
    getLogsOfSwap(selectedSwap, false).then((response) => setSwapLogs(response.logs.map(parseCliLogString)))
  }, [selectedSwap]);

  useEffect(() => {
    if (attachDaemonLogs === false) {
      setDaemonLogs(null);
      return;
    }

    setDaemonLogs(store.getState().rpc?.logs)
  }, [attachDaemonLogs]);

  // Whether to display the log editor
  const [swapLogsEditorOpen, setSwapLogsEditorOpen] = useState(false);
  const [daemonLogsEditorOpen, setDaemonLogsEditorOpen] = useState(false);

  const bodyTooLong = bodyText.length > MAX_FEEDBACK_LENGTH;

  const clearState = () => {
    setBodyText("");
    setAttachDaemonLogs(false);
    setSelectedSwap(null);
  }

  const sendFeedback = async () => {
    if (pending) {
      return;
    }

    try {
      setPending(true);
      await submitFeedback(bodyText, selectedSwap, logsToRawString(swapLogs), logsToRawString(daemonLogs));
      enqueueSnackbar("Feedback submitted successfully!", {
        variant: "success",
      });
      clearState()
    } catch (e) {
      logger.error(`Failed to submit feedback: ${e}`);
      enqueueSnackbar(`Failed to submit feedback (${e})`, {
        variant: "error",
      });
    } finally {
      setPending(false);
    }
    onClose();
  }

  return (
    <Dialog open={open} onClose={onClose}>
      <DialogTitle>Submit Feedback</DialogTitle>
      <DialogContent>
        <ul>
          <li>Got something to say? Drop us a message below. </li>
          <li>If you had an issue with a specific swap, select it from the dropdown to attach the logs.
            It will help us figure out what went wrong.
          </li>
          <li>We appreciate you taking the time to share your thoughts! Every message is read by a core developer!</li>
        </ul>
        <Box
          style={{
            display: "flex",
            flexDirection: "column",
            gap: "1rem",
          }}
        >
          <TextField
            variant="outlined"
            value={bodyText}
            onChange={(e) => setBodyText(e.target.value)}
            label={
              bodyTooLong
                ? `Text is too long (${bodyText.length}/${MAX_FEEDBACK_LENGTH})`
                : "Message"
            }
            multiline
            minRows={4}
            maxRows={4}
            fullWidth
            error={bodyTooLong}
          />
          <Box style={{
            display: "flex",
            flexDirection: "row",
          }}>

            <SwapSelectDropDown
              selectedSwap={selectedSwap}
              setSelectedSwap={setSelectedSwap}
            />
            {selectedSwap !== null ? <IconButton onClick={() => setSwapLogsEditorOpen(true)}>
              <Visibility />
            </IconButton> : <></>
            }
          </Box>
          <LogViewer open={swapLogsEditorOpen} setOpen={setSwapLogsEditorOpen} logs={swapLogs} />
          <Box style={{
            display: "flex",
            flexDirection: "row",
          }}>
            <Paper variant="outlined" style={{ padding: "0.5rem", width: "100%" }} >
              <FormControlLabel
                control={
                  <Checkbox
                    color="primary"
                    checked={attachDaemonLogs}
                    onChange={(e) => setAttachDaemonLogs(e.target.checked)}
                  />
                }
                label="Attach logs from the current session"
              />
            </Paper>
            {attachDaemonLogs ? <IconButton onClick={() => setDaemonLogsEditorOpen(true)}>
              <Visibility />
            </IconButton> : <></>
            }
          </Box>
          <LogViewer open={daemonLogsEditorOpen} setOpen={setDaemonLogsEditorOpen} logs={daemonLogs} />
        </Box>
      </DialogContent>
      <DialogActions>
        <Button onClick={() => { clearState(); onClose() }}>Cancel</Button>
        <LoadingButton
          color="primary"
          variant="contained"
          onClick={sendFeedback}
          loading={pending}
        >
          Submit
        </LoadingButton>
      </DialogActions>
    </Dialog>
  );
}

function LogViewer(
  { open,
    setOpen,
    logs,
  }: {
    open: boolean,
    setOpen: (boolean) => void,
    logs: (string | CliLog)[] | null,
  }) {
  return (
    <Dialog open={open} onClose={() => setOpen(false)} fullWidth>
      <DialogContent>
        <DialogContentText>
          <Typography>
            These are the logs that would be attached to your feedback message and provided to the developers.
            You can search them to check that no information you don't want to reveal gets submitted.
          </Typography>
        </DialogContentText>
        <CliLogsBox label="Logs" logs={logs} />
      </DialogContent>
      <DialogActions>
        <Button variant="contained" color="primary" onClick={() => setOpen(false)}>
          Close
        </Button>
      </DialogActions>
    </Dialog >
  )
}