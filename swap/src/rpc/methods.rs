use crate::bitcoin::bitcoin_address;
use crate::cli::api::request::{
    BalanceArgs, BuyXmrArgs, CancelAndRefundArgs, GetCurrentSwapArgs, GetHistoryArgs,
    GetSwapInfoArgs, ListSellersArgs, MoneroRecoveryArgs, Request, ResumeSwapArgs,
    SuspendCurrentSwapArgs, WithdrawBtcArgs,
};
use crate::cli::api::Context;
use crate::monero::monero_address;
use anyhow::Result;
use jsonrpsee::server::RpcModule;
use jsonrpsee::types::Params;
use libp2p::core::Multiaddr;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;

trait ConvertToJsonRpseeError<T> {
    fn to_jsonrpsee_result(self) -> Result<T, jsonrpsee_core::Error>;
}

impl<T> ConvertToJsonRpseeError<T> for Result<T, anyhow::Error> {
    fn to_jsonrpsee_result(self) -> Result<T, jsonrpsee_core::Error> {
        self.map_err(|e| jsonrpsee_core::Error::Custom(e.to_string()))
    }
}

pub fn register_modules(outer_context: Context) -> Result<RpcModule<Context>> {
    let mut module = RpcModule::new(outer_context);

    module.register_async_method("suspend_current_swap", |_, context| async move {
        SuspendCurrentSwapArgs {}
            .request(context)
            .await
            .to_jsonrpsee_result()
    })?;

    module.register_async_method("get_swap_info", |params_raw, context| async move {
        let params: GetSwapInfoArgs = params_raw.parse()?;

        params.request(context).await.to_jsonrpsee_result()
    })?;

    module.register_async_method("get_bitcoin_balance", |params_raw, context| async move {
        let params: BalanceArgs = params_raw.parse()?;

        params.request(context).await.to_jsonrpsee_result()
    })?;

    module.register_async_method("get_history", |_, context| async move {
        GetHistoryArgs {}
            .request(context)
            .await
            .to_jsonrpsee_result()
    })?;

    module.register_async_method("get_logs", |params_raw, context| async move {
        #[derive(Debug, Clone, Deserialize)]
        struct Params {
            swap_id: Option<Uuid>,
            logs_dir: Option<PathBuf>,
            redact: bool,
        }

        let params: Params = params_raw.parse()?;

        execute_request(
            params_raw,
            Method::Logs {
                swap_id: params.swap_id,
                logs_dir: params.logs_dir,
                redact: params.redact,
            },
            &context,
        )
        .await
    })?;

    module.register_async_method("resume_swap", |params_raw, context| async move {
        let params: ResumeSwapArgs = params_raw.parse()?;

        params.request(context).await.to_jsonrpsee_result()
    })?;

    module.register_async_method("cancel_refund_swap", |params_raw, context| async move {
        let params: CancelAndRefundArgs = params_raw.parse()?;

        params.request(context).await.to_jsonrpsee_result()
    })?;

    module.register_async_method(
        "get_monero_recovery_info",
        |params_raw, context| async move {
            let params: MoneroRecoveryArgs = params_raw.parse()?;

            params.request(context).await.to_jsonrpsee_result()
        },
    )?;

    module.register_async_method("withdraw_btc", |params_raw, context| async move {
        let mut params: WithdrawBtcArgs = params_raw.parse()?;

        params.address =
            bitcoin_address::validate(params.address, context.config.env_config.bitcoin_network)
                .to_jsonrpsee_result()?;

        params.request(context).await.to_jsonrpsee_result()
    })?;

    module.register_async_method("buy_xmr", |params_raw, context| async move {
        let mut params: BuyXmrArgs = params_raw.parse()?;

        params.bitcoin_change_address = bitcoin_address::validate(
            params.bitcoin_change_address,
            context.config.env_config.bitcoin_network,
        )
        .to_jsonrpsee_result()?;

        params.monero_receive_address = monero_address::validate(
            params.monero_receive_address,
            context.config.env_config.monero_network,
        )
        .to_jsonrpsee_result()?;

        params.request(context).await.to_jsonrpsee_result()
    })?;

    module.register_async_method("list_sellers", |params_raw, context| async move {
        let params: ListSellersArgs = params_raw.parse()?;

        params.request(context).await.to_jsonrpsee_result()
    })?;

    module.register_async_method("get_current_swap", |_, context| async move {
        GetCurrentSwapArgs {}
            .request(context)
            .await
            .to_jsonrpsee_result()
    })?;

    Ok(module)
}
