import alertsSlice from "./features/alertsSlice";
import providersSlice from "./features/makersSlice";
import ratesSlice from "./features/ratesSlice";
import rpcSlice from "./features/rpcSlice";
import swapReducer from "./features/swapSlice";
import torSlice from "./features/torSlice";
import settingsSlice from "./features/settingsSlice";
import nodesSlice from "./features/nodesSlice";

export const reducers = {
  swap: swapReducer,
  makers: providersSlice,
  tor: torSlice,
  rpc: rpcSlice,
  alerts: alertsSlice,
  rates: ratesSlice,
  settings: settingsSlice,
  nodes: nodesSlice,
};
