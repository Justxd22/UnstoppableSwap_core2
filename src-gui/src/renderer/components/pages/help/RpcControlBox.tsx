import { Box, makeStyles } from "@material-ui/core";
import FolderOpenIcon from "@material-ui/icons/FolderOpen";
import PlayArrowIcon from "@material-ui/icons/PlayArrow";
import StopIcon from "@material-ui/icons/Stop";
import PromiseInvokeButton from "renderer/components/PromiseInvokeButton";
import { useAppSelector, useIsContextAvailable } from "store/hooks";
import InfoBox from "../../modal/swap/InfoBox";
import CliLogsBox from "../../other/RenderedCliLog";

const useStyles = makeStyles((theme) => ({
  actionsOuter: {
    display: "flex",
    gap: theme.spacing(1),
    alignItems: "center",
  },
}));

export default function RpcControlBox() {
  const isRunning = useIsContextAvailable();
  const classes = useStyles();
  const logs = useAppSelector((s) => s.rpc.logs);

  console.log(logs);

  return (
    <InfoBox
      title={`Daemon Controller`}
      mainContent={
        <CliLogsBox
          label="Swap Daemon Logs (current session only)"
          logs={logs}
        />
      }
      additionalContent={
        <Box className={classes.actionsOuter}>
          <PromiseInvokeButton
            variant="contained"
            endIcon={<PlayArrowIcon />}
            disabled={isRunning}
            onInvoke={() => {
              throw new Error("Not implemented");
            }}
          >
            Start Daemon
          </PromiseInvokeButton>
          <PromiseInvokeButton
            variant="contained"
            endIcon={<StopIcon />}
            disabled={!isRunning}
            onInvoke={() => {
              throw new Error("Not implemented");
            }}
          >
            Stop Daemon
          </PromiseInvokeButton>
          <PromiseInvokeButton
            endIcon={<FolderOpenIcon />}
            isIconButton
            size="small"
            tooltipTitle="Open the data directory of the Swap Daemon in your file explorer"
            onInvoke={() => {
              throw new Error("Not implemented");
            }}
          />
        </Box>
      }
      icon={null}
      loading={false}
    />
  );
}
