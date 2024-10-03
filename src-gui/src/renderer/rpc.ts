import { invoke as invokeUnsafe } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  BalanceArgs,
  BalanceResponse,
  BuyXmrArgs,
  BuyXmrResponse,
  TauriLogEvent,
  GetLogsArgs,
  GetLogsResponse,
  GetSwapInfoResponse,
  ListSellersArgs,
  MoneroRecoveryArgs,
  ResumeSwapArgs,
  ResumeSwapResponse,
  SuspendCurrentSwapResponse,
  TauriContextStatusEvent,
  TauriSwapProgressEventWrapper,
  WithdrawBtcArgs,
  WithdrawBtcResponse,
  TauriDatabaseStateEvent,
  TauriTimelockChangeEvent,
} from "models/tauriModel";
import {
  contextStatusEventReceived,
  receivedCliLog,
  databaseStateEventReceived,
  rpcSetBalance,
  rpcSetSwapInfo,
} from "store/features/rpcSlice";
import { swapProgressEventReceived } from "store/features/swapSlice";
import { store } from "./store/storeRenderer";
import { Provider } from "models/apiModel";
import { providerToConcatenatedMultiAddr } from "utils/multiAddrUtils";
import { MoneroRecoveryResponse } from "models/rpcModel";
import { ListSellersResponse } from "../models/tauriModel";

export async function initEventListeners() {
  // This operation is in-expensive
  // We do this in case we miss the context init progress event because the frontend took too long to load
  // TOOD: Replace this with a more reliable mechanism (such as an event replay mechanism)
  if (await checkContextAvailability()) {
    store.dispatch(contextStatusEventReceived({ type: "Available" }));
  }

  listen<TauriSwapProgressEventWrapper>("swap-progress-update", (event) => {
    console.log("Received swap progress event", event.payload);
    store.dispatch(swapProgressEventReceived(event.payload));
  });

  listen<TauriContextStatusEvent>("context-init-progress-update", (event) => {
    console.log("Received context init progress event", event.payload);
    store.dispatch(contextStatusEventReceived(event.payload));
  });

  listen<TauriLogEvent>("cli-log-emitted", (event) => {
    console.log("Received cli log event", event.payload);
    store.dispatch(receivedCliLog(event.payload));
  });

  listen<TauriDatabaseStateEvent>("swap-database-state-update", (event) => {
    console.log("Received swap database state update event", event.payload);
    store.dispatch(databaseStateEventReceived(event.payload));
  });

  listen<TauriTimelockChangeEvent>('timelock-change', (event) => {
    console.log('Received timelock change event', event.payload);
    store.dispatch(timelockChangeEventReceived(event.payload));
  })
}

async function invoke<ARGS, RESPONSE>(
  command: string,
  args: ARGS,
): Promise<RESPONSE> {
  return invokeUnsafe(command, {
    args: args as Record<string, unknown>,
  }) as Promise<RESPONSE>;
}

async function invokeNoArgs<RESPONSE>(command: string): Promise<RESPONSE> {
  return invokeUnsafe(command) as Promise<RESPONSE>;
}

export async function checkBitcoinBalance() {
  const response = await invoke<BalanceArgs, BalanceResponse>("get_balance", {
    force_refresh: true,
  });

  store.dispatch(rpcSetBalance(response.balance));
}

export async function getAllSwapInfos() {
  const response =
    await invokeNoArgs<GetSwapInfoResponse[]>("get_swap_infos_all");

  response.forEach((swapInfo) => {
    store.dispatch(rpcSetSwapInfo(swapInfo));
  });
}

export async function withdrawBtc(address: string): Promise<string> {
  const response = await invoke<WithdrawBtcArgs, WithdrawBtcResponse>(
    "withdraw_btc",
    {
      address,
      amount: null,
    },
  );

  return response.txid;
}

export async function buyXmr(
  seller: Provider,
  bitcoin_change_address: string | null,
  monero_receive_address: string,
) {
  await invoke<BuyXmrArgs, BuyXmrResponse>(
    "buy_xmr",
    bitcoin_change_address == null
      ? {
          seller: providerToConcatenatedMultiAddr(seller),
          monero_receive_address,
        }
      : {
          seller: providerToConcatenatedMultiAddr(seller),
          monero_receive_address,
          bitcoin_change_address,
        },
  );
}

export async function resumeSwap(swapId: string) {
  await invoke<ResumeSwapArgs, ResumeSwapResponse>("resume_swap", {
    swap_id: swapId,
  });
}

export async function suspendCurrentSwap() {
  await invokeNoArgs<SuspendCurrentSwapResponse>("suspend_current_swap");
}

export async function getMoneroRecoveryKeys(
  swapId: string,
): Promise<MoneroRecoveryResponse> {
  return await invoke<MoneroRecoveryArgs, MoneroRecoveryResponse>(
    "monero_recovery",
    {
      swap_id: swapId,
    },
  );
}

export async function checkContextAvailability(): Promise<boolean> {
  const available = await invokeNoArgs<boolean>("is_context_available");
  return available;
}

export async function getLogsOfSwap(
  swapId: string,
  redact: boolean,
): Promise<GetLogsResponse> {
  return await invoke<GetLogsArgs, GetLogsResponse>("get_logs", {
    swap_id: swapId,
    redact,
  });
}

export async function listSellersAtRendezvousPoint(
  rendezvousPointAddress: string,
): Promise<ListSellersResponse> {
  return await invoke<ListSellersArgs, ListSellersResponse>("list_sellers", {
    rendezvous_point: rendezvousPointAddress,
  });
}
function timelockChangeEventReceived(payload: TauriTimelockChangeEvent): any {
  throw new Error("Function not implemented.");
}

