import {
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableRow,
  Typography,
  IconButton,
  Box,
  makeStyles,
  Tooltip,
  Select,
  MenuItem,
  TableHead,
  Paper,
  Button,
  Dialog,
  DialogContent,
  DialogActions,
  DialogTitle,
  useTheme,
  Switch,
} from "@material-ui/core";
import InfoBox from "renderer/components/modal/swap/InfoBox";
import {
  addRendezvousPoint,
  removeNode,
  removeRendezvousPoint,
  resetSettings,
  setFetchFiatPrices,
  setFiatCurrency,
} from "store/features/settingsSlice";
import {
  addNode,
  Blockchain,
  FiatCurrency,
  moveUpNode,
  Network,
  setTheme,
} from "store/features/settingsSlice";
import { useAppDispatch, useAppSelector, useNodes, useSettings } from "store/hooks";
import ValidatedTextField from "renderer/components/other/ValidatedTextField";
import HelpIcon from '@material-ui/icons/HelpOutline';
import { ReactNode, useState } from "react";
import { Theme } from "renderer/components/theme";
import { Add, ArrowUpward, Delete, Edit, HourglassEmpty } from "@material-ui/icons";
import { getNetwork } from "store/config";
import { currencySymbol } from "utils/formatUtils";
import { isValidMultiAddressWithPeerId } from "utils/parseUtils";

const PLACEHOLDER_ELECTRUM_RPC_URL = "ssl://blockstream.info:700";
const PLACEHOLDER_MONERO_NODE_URL = "http://xmr-node.cakewallet.com:18081";

const useStyles = makeStyles((theme) => ({
  title: {
    display: "flex",
    alignItems: "center",
    gap: theme.spacing(1),
  }
}));

/**
 * The settings box, containing the settings for the GUI.
 */
export default function SettingsBox() {
  const classes = useStyles();
  const theme = useTheme();

  return (
    <InfoBox
      title={
        <Box className={classes.title}>
          Settings
        </Box>
      }
      mainContent={
        <Typography variant="subtitle2">
          Customize the settings of the GUI.
          Some of these require a restart to take effect.
        </Typography>
      }
      additionalContent={
        <>
          {/* Table containing the settings */}
          <TableContainer>
            <Table>
              <TableBody>
                <ElectrumRpcUrlSetting />
                <MoneroNodeUrlSetting />
                <RendezvousPointsSetting />
                <FetchFiatPricesSetting />
                <ThemeSetting />
              </TableBody>
            </Table>
          </TableContainer>
          {/* Reset button with a bit of spacing */}
          <Box mt={theme.spacing(0.1)} />
          <ResetButton />
        </>
      }
      icon={null}
      loading={false}
    />
  );
}

/**
 * A button that allows you to reset the settings. 
 * Opens a modal that asks for confirmation first.
 */
function ResetButton() {
  const dispatch = useAppDispatch();
  const [modalOpen, setModalOpen] = useState(false);

  const onReset = () => {
    dispatch(resetSettings());
    setModalOpen(false);
  };

  return (
    <>
      <Button variant="outlined" onClick={() => setModalOpen(true)}>Reset Settings</Button>
      <Dialog open={modalOpen} onClose={() => setModalOpen(false)}>
        <DialogTitle>Reset Settings</DialogTitle>
        <DialogContent>Are you sure you want to reset the settings?</DialogContent>
        <DialogActions>
          <Button onClick={() => setModalOpen(false)}>Cancel</Button>
          <Button color="primary" onClick={onReset}>Reset</Button>
        </DialogActions>
      </Dialog>
    </>
  )
}

/**
 * A setting that allows you to enable or disable the fetching of fiat prices.
 */
function FetchFiatPricesSetting() {
  const fetchFiatPrices = useSettings((s) => s.fetchFiatPrices);
  const dispatch = useAppDispatch();

  return (
    <>
      <TableRow>
        <TableCell>
          <SettingLabel label="Query fiat prices" tooltip="Whether to fetch fiat prices via the clearnet. This is required for the price display to work. If you require total anonymity and don't use a VPN, you should disable this." />
        </TableCell>
        <TableCell>
          <Switch
            color="primary"
            checked={fetchFiatPrices}
            onChange={(event) => dispatch(setFetchFiatPrices(event.currentTarget.checked))}
          />
        </TableCell>
      </TableRow>
      {fetchFiatPrices ? <FiatCurrencySetting /> : <></>}
    </>
  );
}

/**
 * A setting that allows you to select the fiat currency to display prices in.
 */
function FiatCurrencySetting() {
  const fiatCurrency = useSettings((s) => s.fiatCurrency);
  const dispatch = useAppDispatch();
  const onChange = (e: React.ChangeEvent<{ value: unknown }>) =>
    dispatch(setFiatCurrency(e.target.value as FiatCurrency));

  return (
    <TableRow>
      <TableCell>
        <SettingLabel label="Fiat currency" tooltip="This is the currency that the price display will show prices in." />
      </TableCell>
      <TableCell>
        <Select
          value={fiatCurrency}
          onChange={onChange}
          variant="outlined"
          fullWidth
        >
          {Object.values(FiatCurrency).map((currency) => (
            <MenuItem key={currency} value={currency}>
              <Box sx={{ display: 'flex', justifyContent: 'space-between', width: '100%' }}>
                <Box>{currency}</Box>
                <Box>{currencySymbol(currency)}</Box>
              </Box>
            </MenuItem>
          ))}
        </Select>
      </TableCell>
    </TableRow>
  );
}

/**
 * URL validation function, forces the URL to be in the format of "protocol://host:port/"
 */
function isValidUrl(url: string, allowedProtocols: string[]): boolean {
  const urlPattern = new RegExp(`^(${allowedProtocols.join("|")})://[^\\s]+:\\d+/?$`);
  return urlPattern.test(url);
}

/**
 * A setting that allows you to select the Electrum RPC URL to use.
 */
function ElectrumRpcUrlSetting() {
  const [tableVisible, setTableVisible] = useState(false);
  const network = getNetwork();

  const isValid = (url: string) => isValidUrl(url, ["ssl", "tcp"]);

  return (
    <TableRow>
      <TableCell>
        <SettingLabel label="Electrum Server" tooltip="Edit a list of Electrum servers to use. If left empty, the GUI will choose from a list of known servers at random." />
      </TableCell>
      <TableCell>
        <IconButton
          onClick={() => setTableVisible(true)}
        >
          {<Edit />}
        </IconButton>
        {tableVisible ? <NodeTableModal
          open={tableVisible}
          onClose={() => setTableVisible(false)}
          network={network}
          blockchain={Blockchain.Bitcoin}
          isValid={isValid}
          placeholder={PLACEHOLDER_ELECTRUM_RPC_URL}
        /> : <></>}
      </TableCell>
    </TableRow>
  );
}

/**
 * A label for a setting, with a tooltip icon.
 */
function SettingLabel({ label, tooltip }: { label: ReactNode, tooltip: string | null }) {
  return <Box style={{ display: "flex", alignItems: "center", gap: "0.5rem" }}>
    <Box>
      {label}
    </Box>
    <Tooltip title={tooltip}>
      <IconButton size="small">
        <HelpIcon />
      </IconButton>
    </Tooltip>
  </Box>
}

/**
 * A setting that allows you to select the Monero Node URL to use.
 */
function MoneroNodeUrlSetting() {
  const network = getNetwork();
  const [tableVisible, setTableVisible] = useState(false);

  const isValid = (url: string) => isValidUrl(url, ["http"]);

  return (
    <TableRow>
      <TableCell>
        <SettingLabel label="Monero Node" tooltip="Edit a list of Monero nodes to use. If left empty, the GUI will choose from a list of known nodes at random." />
      </TableCell>
      <TableCell>
        <IconButton
          onClick={() => setTableVisible(!tableVisible)}
        >
          <Edit />
        </IconButton>
        {tableVisible ? <NodeTableModal
          open={tableVisible}
          onClose={() => setTableVisible(false)}
          network={network}
          blockchain={Blockchain.Monero}
          isValid={isValid}
          placeholder={PLACEHOLDER_MONERO_NODE_URL}
        /> : <></>}
      </TableCell>
    </TableRow>
  );
}

/**
 * A setting that allows you to select the theme of the GUI.
 */
function ThemeSetting() {
  const theme = useAppSelector((s) => s.settings.theme);
  const dispatch = useAppDispatch();

  return (
    <TableRow>
      <TableCell>
        <SettingLabel label="Theme" tooltip="This is the theme of the GUI." />
      </TableCell>
      <TableCell>
        <Select
          value={theme}
          onChange={(e) => dispatch(setTheme(e.target.value as Theme))}
          variant="outlined"
          fullWidth
        >
          {/** Create an option for each theme variant */}
          {Object.values(Theme).map((themeValue) => (
            <MenuItem key={themeValue} value={themeValue}>
              {themeValue.charAt(0).toUpperCase() + themeValue.slice(1)}
            </MenuItem>
          ))}
        </Select>
      </TableCell>
    </TableRow>
  );
}

/**
 * A modal containing a NodeTable for a given network and blockchain.
 * It allows you to add, remove, and move nodes up the list.
 */
function NodeTableModal({
  open,
  onClose,
  network,
  isValid,
  placeholder,
  blockchain
}: {
  network: Network;
  blockchain: Blockchain;
  isValid: (url: string) => boolean;
  placeholder: string;
  open: boolean;
  onClose: () => void;
}) {
  return (
    <Dialog open={open} onClose={onClose}>
      <DialogTitle>Available Nodes</DialogTitle>
      <DialogContent>
        <Typography variant="subtitle2">
          When the daemon is started, it will attempt to connect to the first available {blockchain} node in this list.
          If you leave this field empty or all nodes are unavailable, it will choose from a list of known nodes at random.
          Requires a restart to take effect.
        </Typography>
        <NodeTable network={network} blockchain={blockchain} isValid={isValid} placeholder={placeholder} />
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose} size="large">Close</Button>
      </DialogActions>
    </Dialog>
  )
}

// Create a circle SVG with a given color and radius
function Circle({ color, radius = 6 }: { color: string, radius?: number }) {
  return <span>
    <svg width={radius * 2} height={radius * 2} viewBox={`0 0 ${radius * 2} ${radius * 2}`}>
      <circle cx={radius} cy={radius} r={radius} fill={color} />
    </svg>
  </span>
}

/**
 * Displays a status indicator for a node
 */
function NodeStatus({ status }: { status: boolean | undefined }) {
  const theme = useTheme();

  switch (status) {
    case true:
      return <Tooltip title={"This node is available and responding to RPC requests"}>
        <Circle color={theme.palette.success.dark} />
      </Tooltip>;
    case false:
      return <Tooltip title={"This node is not available or not responding to RPC requests"}>
        <Circle color={theme.palette.error.dark} />
      </Tooltip>;
    default:
      return <Tooltip title={"The status of this node is currently unknown"}>
        <HourglassEmpty />
      </Tooltip>;
  }
}

/**
 * A table that displays the available nodes for a given network and blockchain.
 * It allows you to add, remove, and move nodes up the list.
 * It fetches the nodes from the store (nodesSlice) and the statuses of all nodes every 15 seconds.
 */
function NodeTable({
  network,
  blockchain,
  isValid,
  placeholder,
}: {
  network: Network,
  blockchain: Blockchain,
  isValid: (url: string) => boolean,
  placeholder: string,
}) {
  const availableNodes = useSettings((s) => s.nodes[network][blockchain]);
  const currentNode = availableNodes[0];
  const nodeStatuses = useNodes((s) => s.nodes);
  const [newNode, setNewNode] = useState("");
  const dispatch = useAppDispatch();

  const onAddNewNode = () => {
    dispatch(addNode({ network, type: blockchain, node: newNode }));
    setNewNode("");
  }

  const onRemoveNode = (node: string) =>
    dispatch(removeNode({ network, type: blockchain, node }));

  const onMoveUpNode = (node: string) =>
    dispatch(moveUpNode({ network, type: blockchain, node }));

  const moveUpButton = (node: string) => {
    if (currentNode === node)
      return <></>;

    return (
      <Tooltip title={"Move this node to the top of the list"}>
        <IconButton onClick={() => onMoveUpNode(node)}>
          <ArrowUpward />
        </IconButton>
      </Tooltip>
    )
  }

  return (
    <TableContainer component={Paper} style={{ marginTop: '1rem' }} elevation={0}>
      <Table size="small">
        {/* Table header row */}
        <TableHead>
          <TableRow>
            <TableCell align="center">Node URL</TableCell>
            <TableCell align="center">Status</TableCell>
            <TableCell align="center">Actions</TableCell>
          </TableRow>
        </TableHead>
        <TableBody>
          {/* Table body rows: one for each node */}
          {availableNodes.map((node, index) => (
            <TableRow key={index}>
              {/* Node URL */}
              <TableCell>
                <Typography variant="overline">{node}</Typography>
              </TableCell>
              {/* Node status icon */}
              <TableCell align="center">
                <NodeStatus status={nodeStatuses[blockchain][node]} />
              </TableCell>
              {/* Remove and move buttons */}
              <TableCell>
                <Box style={{ display: "flex" }}>
                  <Tooltip
                    title={"Remove this node from your list"}
                    children={<IconButton
                      onClick={() => onRemoveNode(node)}
                      children={<Delete />}
                    />}
                  />
                  {moveUpButton(node)}
                </Box>
              </TableCell>
            </TableRow>
          ))}
          {/* Last row: add a new node */}
          <TableRow key={-1}>
            <TableCell>
              <ValidatedTextField
                label="Add a new node"
                value={newNode}
                onValidatedChange={setNewNode}
                placeholder={placeholder}
                fullWidth
                isValid={isValid}
                variant="outlined"
                noErrorWhenEmpty
              />
            </TableCell>
            <TableCell></TableCell>
            <TableCell>
              <Tooltip title={"Add this node to your list"}>
                <IconButton onClick={onAddNewNode} disabled={availableNodes.includes(newNode) || newNode.length === 0}>
                  <Add />
                </IconButton>
              </Tooltip>
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </TableContainer>
  )
}

/**
 * A setting that allows you to manage rendezvous points for maker discovery
 */
function RendezvousPointsSetting() {
  const [tableVisible, setTableVisible] = useState(false);
  const rendezvousPoints = useSettings((s) => s.rendezvousPoints);
  const dispatch = useAppDispatch();
  const [newPoint, setNewPoint] = useState("");

  const onAddNewPoint = () => {
    dispatch(addRendezvousPoint(newPoint));
    setNewPoint("");
  };

  const onRemovePoint = (point: string) => {
    dispatch(removeRendezvousPoint(point));
  };

  return (
    <TableRow>
      <TableCell>
        <SettingLabel 
          label="Rendezvous Points" 
          tooltip="These are the points where makers can be discovered. Add custom rendezvous points here to expand your maker discovery options." 
        />
      </TableCell>
      <TableCell>
        <IconButton onClick={() => setTableVisible(true)}>
          <Edit />
        </IconButton>
        {tableVisible && (
          <Dialog open={true} onClose={() => setTableVisible(false)} maxWidth="sm">
            <DialogTitle>Rendezvous Points</DialogTitle>
            <DialogContent>
              <Typography variant="subtitle2">
                Add or remove rendezvous points where makers can be discovered. 
                These points help you find trading partners in a decentralized way.
              </Typography>
              <TableContainer component={Paper} style={{ marginTop: '1rem' }} elevation={0}>
                <Table size="small">
                  <TableHead>
                    <TableRow>
                      <TableCell>Rendezvous Point Address</TableCell>
                      <TableCell align="right">Actions</TableCell>
                    </TableRow>
                  </TableHead>
                  <TableBody>
                    {rendezvousPoints.map((point, index) => (
                      <TableRow key={index}>
                        <TableCell style={{ wordBreak: 'break-all' }}>
                          <Typography variant="overline">{point}</Typography>
                        </TableCell>
                        <TableCell align="right">
                          <Tooltip title="Remove this rendezvous point">
                            <IconButton onClick={() => onRemovePoint(point)}>
                              <Delete />
                            </IconButton>
                          </Tooltip>
                        </TableCell>
                      </TableRow>
                    ))}
                    <TableRow>
                      <TableCell>
                        <ValidatedTextField
                          label="Add new rendezvous point"
                          value={newPoint}
                          onValidatedChange={setNewPoint}
                          placeholder="/dns4/discover.unstoppableswap.net/tcp/8888/p2p/12D3KooWA6cnqJpVnreBVnoro8midDL9Lpzmg8oJPoAGi7YYaamE"
                          fullWidth
                          isValid={isValidMultiAddressWithPeerId}
                          variant="outlined"
                          noErrorWhenEmpty
                        />
                      </TableCell>
                      <TableCell align="right">
                        <Tooltip title="Add this rendezvous point">
                          <IconButton 
                            onClick={onAddNewPoint}
                            disabled={!isValidMultiAddressWithPeerId(newPoint) || newPoint.length === 0}
                          >
                            <Add />
                          </IconButton>
                        </Tooltip>
                      </TableCell>
                    </TableRow>
                  </TableBody>
                </Table>
              </TableContainer>
            </DialogContent>
            <DialogActions>
              <Button onClick={() => setTableVisible(false)} size="large">
                Close
              </Button>
            </DialogActions>
          </Dialog>
        )}
      </TableCell>
    </TableRow>
  );
}