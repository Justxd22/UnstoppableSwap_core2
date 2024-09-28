import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { PersistGate } from "redux-persist/integration/react";
import { setAlerts } from "store/features/alertsSlice";
import {
  registryConnectionFailed,
  setRegistryProviders,
} from "store/features/providersSlice";
import { setBtcPrice, setXmrPrice } from "store/features/ratesSlice";
import { setBitcoinConfirmationTarget } from "store/features/settingsSlice";
import logger from "../utils/logger";
import {
  fetchAlertsViaHttp,
  fetchBtcPrice,
  fetchProvidersViaHttp,
  fetchXmrPrice,
} from "./api";
import App from "./components/App";
import { initEventListeners } from "./rpc";
import { persistor, store } from "./store/storeRenderer";

const container = document.getElementById("root");
const root = createRoot(container!);
root.render(
  <Provider store={store}>
    <PersistGate loading={null} persistor={persistor}>
      <App />
    </PersistGate>
  </Provider>,
);

async function fetchInitialData() {
  try {
    const providerList = await fetchProvidersViaHttp();
    store.dispatch(setRegistryProviders(providerList));

    logger.info(
      { providerList },
      "Fetched providers via UnstoppableSwap HTTP API",
    );
  } catch (e) {
    store.dispatch(registryConnectionFailed());
    logger.error(e, "Failed to fetch providers via UnstoppableSwap HTTP API");
  }

  try {
    const alerts = await fetchAlertsViaHttp();
    store.dispatch(setAlerts(alerts));
    logger.info({ alerts }, "Fetched alerts via UnstoppableSwap HTTP API");
  } catch (e) {
    logger.error(e, "Failed to fetch alerts via UnstoppableSwap HTTP API");
  }

  try {
    const xmrPrice = await fetchXmrPrice();
    store.dispatch(setXmrPrice(xmrPrice));
    logger.info({ xmrPrice }, "Fetched XMR price");

    const btcPrice = await fetchBtcPrice();
    store.dispatch(setBtcPrice(btcPrice));
    logger.info({ btcPrice }, "Fetched BTC price");
  } catch (e) {
    logger.error(e, "Error retrieving fiat prices");
  }
}

fetchInitialData();
initEventListeners();

setInterval(() => {
  const n = Math.floor(Math.random() * 100);
  console.log(
    `previous was ${store.getState().settings.bitcoinConfirmationTarget} will set to ${n}`,
  );
  store.dispatch(setBitcoinConfirmationTarget(n));
}, 10000);
