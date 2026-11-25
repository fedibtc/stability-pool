//! The module's `fedimint-cli` commands, behind the `cli` feature.

use std::{ffi, iter};

use anyhow::bail;
use clap::{Parser, ValueEnum};
use fedi_stability_pool_common::{
    BtcTransferOutput, BtcTransferRequest, FiatAmount, TransferRequest,
};
use rand::Rng;
use serde::de::DeserializeOwned;
use tracing::info;

use super::*;

impl StabilityPoolClientModule {
    pub(crate) async fn handle_cli_command_inner(
        &self,
        args: &[ffi::OsString],
    ) -> anyhow::Result<serde_json::Value> {
        let command = CliCommand::parse_from(
            iter::once(&ffi::OsString::from("stability-pool")).chain(args.iter()),
        );

        match command {
            CliCommand::Pubkey { account_type } => Ok(serde_json::to_value(
                self.own_keypair(account_type.into()).public_key(),
            )?),

            CliCommand::AccountInfo { account_type } => {
                let sync_service = StabilityPoolSyncService::new(
                    self.module_api.clone(),
                    self.db.clone(),
                    self.own_account(account_type.into()).id(),
                )
                .await;
                let mut update_stream = sync_service.subscribe_to_updates();
                sync_service.update_once().await?;
                let sync_response = update_stream
                    .next()
                    .await
                    .unwrap()
                    .expect("must be present after calling update once");
                Ok(serde_json::to_value(sync_response.value)?)
            }

            CliCommand::ActiveDeposits { account_type } => {
                let account_id = self.own_account(account_type.into()).id();
                let active_deposits = self.active_deposits(account_id).await?;
                Ok(serde_json::to_value(active_deposits)?)
            }

            CliCommand::AccountHistory {
                account_type,
                start,
                end,
            } => {
                let account_id = self.own_account(account_type.into()).id();
                let history = self
                    .module_api
                    .account_history(account_id, start..end)
                    .await?;
                Ok(serde_json::to_value(history)?)
            }

            CliCommand::DepositToSeek { amount_msats } => {
                let operation_id = self.deposit_to_seek(amount_msats, ()).await?;
                let mut updates = self
                    .subscribe_deposit_operation(operation_id)
                    .await?
                    .into_stream();

                while let Some(update) = updates.next().await {
                    match update {
                        StabilityPoolDepositOperationState::TxRejected(e) => {
                            bail!("TX rejected: {e}")
                        }
                        StabilityPoolDepositOperationState::PrimaryOutputError(e) => {
                            bail!("Change output error: {e}")
                        }
                        _ => info!("Update: {:?}", update),
                    }
                }

                Ok(serde_json::Value::String(
                    "deposit-to-seek success".to_string(),
                ))
            }

            CliCommand::DepositToProvide {
                amount_msats,
                fee_rate,
            } => {
                let operation_id = self.deposit_to_provide(amount_msats, fee_rate, ()).await?;
                let mut updates = self
                    .subscribe_deposit_operation(operation_id)
                    .await?
                    .into_stream();

                while let Some(update) = updates.next().await {
                    match update {
                        StabilityPoolDepositOperationState::TxRejected(e) => {
                            bail!("TX rejected: {e}")
                        }
                        StabilityPoolDepositOperationState::PrimaryOutputError(e) => {
                            bail!("Change output error: {e}")
                        }
                        _ => info!("Update: {:?}", update),
                    }
                }

                Ok(serde_json::Value::String(
                    "deposit-to-provide success".to_string(),
                ))
            }

            CliCommand::DepositToBtcBalance {
                amount_msats,
                metadata,
            } => {
                let operation_id = self
                    .deposit_to_btc_balance(
                        self.own_account(OwnAccount::FeeRemittance).id(),
                        amount_msats,
                        metadata,
                        (),
                    )
                    .await?;
                let mut updates = self
                    .subscribe_deposit_operation(operation_id)
                    .await?
                    .into_stream();

                while let Some(update) = updates.next().await {
                    match update {
                        StabilityPoolDepositOperationState::TxRejected(e) => {
                            bail!("TX rejected: {e}")
                        }
                        StabilityPoolDepositOperationState::PrimaryOutputError(e) => {
                            bail!("Change output error: {e}")
                        }
                        _ => info!("Update: {:?}", update),
                    }
                }

                Ok(serde_json::Value::String(
                    "deposit-to-btc-balance success".to_string(),
                ))
            }

            CliCommand::Withdraw {
                account_type,
                amount,
            } => {
                let (operation_id, _) = self.withdraw(account_type.into(), amount, ()).await?;
                let mut updates = self.subscribe_withdraw(operation_id).await?.into_stream();

                while let Some(update) = updates.next().await {
                    match update {
                        StabilityPoolWithdrawalOperationState::UnlockTxRejected(e) => {
                            bail!("Unlock TX rejected: {e}")
                        }
                        StabilityPoolWithdrawalOperationState::UnlockProcessingError(e) => {
                            bail!("Unlock processing error: {e}")
                        }
                        StabilityPoolWithdrawalOperationState::WithdrawalTxRejected(e) => {
                            bail!("Withdrawal TX rejected: {e}")
                        }
                        StabilityPoolWithdrawalOperationState::PrimaryOutputError(e) => {
                            bail!("Primary output error: {e}")
                        }
                        _ => info!("Update: {:?}", update),
                    }
                }

                Ok(serde_json::Value::String("withdraw success".to_string()))
            }

            CliCommand::SignTransfer {
                account_type,
                request,
            } => {
                let request_id = match account_type {
                    AccountTypeArg::BtcDepositor => {
                        let request: BtcTransferRequest = parse_json_value(&request)?;
                        TransferRequestId::from(&request)
                    }
                    AccountTypeArg::Seeker | AccountTypeArg::Provider => {
                        let request: TransferRequest = parse_json_value(&request)?;
                        TransferRequestId::from(&request)
                    }
                };
                Ok(serde_json::to_value(
                    self.sign_transfer(account_type.into(), request_id),
                )?)
            }

            CliCommand::SimpleTransfer {
                account_type,
                to_account,
                amount,
                new_fee_rate,
            } => {
                let from = self.own_account(account_type.into());
                let nonce = rand::thread_rng().r#gen();
                match account_type {
                    AccountTypeArg::BtcDepositor => {
                        ensure!(
                            new_fee_rate.is_none(),
                            "Fee rate only applies to provider-to-provider transfer"
                        );
                        let request = BtcTransferRequest::new(
                            nonce,
                            from,
                            amount.parse()?,
                            to_account,
                            vec![],
                            u64::MAX,
                        )?;
                        let signature = self
                            .sign_transfer(account_type.into(), TransferRequestId::from(&request));
                        Ok(serde_json::to_value(SignedBtcTransferRequest::new(
                            request,
                            BTreeMap::from([(0, signature)]),
                        )?)?)
                    }
                    AccountTypeArg::Seeker | AccountTypeArg::Provider => {
                        let request = TransferRequest::new(
                            nonce,
                            from,
                            parse_json_value(&amount)?,
                            to_account,
                            vec![],
                            u64::MAX,
                            new_fee_rate,
                        )?;
                        let signature = self
                            .sign_transfer(account_type.into(), TransferRequestId::from(&request));
                        Ok(serde_json::to_value(SignedTransferRequest::new(
                            request,
                            BTreeMap::from([(0, signature)]),
                        )?)?)
                    }
                }
            }

            CliCommand::Transfer {
                account_type,
                request,
            } => {
                let output = match account_type {
                    AccountTypeArg::BtcDepositor => StabilityPoolOutput::V2(
                        StabilityPoolOutputV2::BtcTransfer(BtcTransferOutput {
                            signed_request: parse_json_value(&request)?,
                        }),
                    ),
                    AccountTypeArg::Seeker | AccountTypeArg::Provider => {
                        StabilityPoolOutput::V0(StabilityPoolOutputV0::Transfer(TransferOutput {
                            signed_request: parse_json_value(&request)?,
                        }))
                    }
                };
                let operation_id = self
                    .submit_transfer(output, account_type.into(), ())
                    .await?;
                let mut updates = self
                    .subscribe_transfer_operation(operation_id)
                    .await?
                    .into_stream();

                while let Some(update) = updates.next().await {
                    match update {
                        StabilityPoolTransferOperationState::TxRejected(e) => {
                            bail!("TX rejected: {e}")
                        }
                        _ => info!("Update: {:?}", update),
                    }
                }

                Ok(serde_json::Value::String("transfer success".to_string()))
            }

            CliCommand::WithdrawIdleBalance {
                account_type,
                amount_msats,
            } => {
                let (operation_id, _) = self
                    .withdraw_idle_balance(account_type.into(), amount_msats, ())
                    .await?;
                let mut updates = self
                    .subscribe_withdraw_idle_balance(operation_id)
                    .await?
                    .into_stream();

                while let Some(update) = updates.next().await {
                    match update {
                        StabilityPoolWithdrawalOperationState::WithdrawalTxRejected(e) => {
                            bail!("Withdrawal TX rejected: {e}")
                        }
                        StabilityPoolWithdrawalOperationState::PrimaryOutputError(e) => {
                            bail!("Primary output error: {e}")
                        }
                        _ => info!("Update: {:?}", update),
                    }
                }

                Ok(serde_json::Value::String(
                    "withdraw idle balance success".to_string(),
                ))
            }
        }
    }
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, ValueEnum)]
pub enum AccountTypeArg {
    Seeker,
    Provider,
    BtcDepositor,
}

impl From<AccountTypeArg> for OwnAccount {
    fn from(arg: AccountTypeArg) -> Self {
        match arg {
            AccountTypeArg::Seeker => OwnAccount::Seeker,
            AccountTypeArg::Provider => OwnAccount::Provider,
            AccountTypeArg::BtcDepositor => OwnAccount::FeeRemittance,
        }
    }
}

fn parse_withdrawal_amount(s: &str) -> Result<FiatOrAll, String> {
    match s.to_lowercase().as_str() {
        "all" => Ok(FiatOrAll::All),
        amount => amount
            .parse::<u64>()
            .map(|fiat| FiatOrAll::Fiat(FiatAmount(fiat)))
            .map_err(|_| format!("Invalid withdrawal amount {amount}, must be \"all\" or a u64")),
    }
}

fn parse_json_value<T: DeserializeOwned>(s: &str) -> Result<T, serde_json::Error> {
    serde_json::from_str(s)
}

#[derive(Parser, Debug, Serialize)]
pub enum CliCommand {
    /// Get the public key for this client's account type
    Pubkey {
        #[arg(long, value_enum, default_value = "seeker")]
        account_type: AccountTypeArg,
    },
    /// Get account info for seeker or provider account
    AccountInfo {
        #[arg(value_enum)]
        account_type: AccountTypeArg,
    },
    /// Get active deposits (staged and locked) for seeker or provider account
    ActiveDeposits {
        #[arg(value_enum)]
        account_type: AccountTypeArg,
    },
    /// Get account history for seeker, provider, or btc-balance account
    AccountHistory {
        #[arg(value_enum)]
        account_type: AccountTypeArg,
        /// Inclusive start index
        start: u64,
        /// Exclusive end index
        end: u64,
    },
    /// Deposit amount to seek liquidity
    DepositToSeek {
        /// Amount in msats to deposit
        amount_msats: Amount,
    },
    /// Deposit amount to provide liquidity
    DepositToProvide {
        /// Amount in msats to deposit
        amount_msats: Amount,
        /// Fee rate in parts per billion
        #[arg(value_parser = parse_json_value::<FeeRate>)]
        fee_rate: FeeRate,
    },
    /// Deposit amount to a btc-balance account with attached metadata
    DepositToBtcBalance {
        /// Amount in msats to deposit
        amount_msats: Amount,
        /// Metadata bytes encoded as JSON, for example: `[1,2,3]`
        #[arg(value_parser = parse_json_value::<BtcBalanceDepositMetadata>)]
        metadata: BtcBalanceDepositMetadata,
    },
    /// Withdraw from seeker or provider account
    Withdraw {
        #[arg(value_enum)]
        account_type: AccountTypeArg,
        #[arg(value_parser = parse_withdrawal_amount)]
        amount: FiatOrAll,
    },
    /// Sign a transfer request (BTC-depositor requests use msats)
    SignTransfer {
        #[arg(long, value_enum, default_value = "seeker")]
        account_type: AccountTypeArg,
        /// Request JSON in the selected account type's format
        request: String,
    },
    /// Create a signed transfer request; amounts are fiat units, or msats for
    /// BTC
    SimpleTransfer {
        #[arg(long, value_enum, default_value = "seeker")]
        account_type: AccountTypeArg,
        to_account: AccountId,
        amount: String,
        #[arg(long, value_parser = parse_json_value::<FeeRate>, required_if_eq("account_type", "provider"))]
        new_fee_rate: Option<FeeRate>,
    },
    /// Submit a signed transfer request
    Transfer {
        #[arg(long, value_enum, default_value = "seeker")]
        account_type: AccountTypeArg,
        /// Signed request JSON in the selected account type's format
        request: String,
    },
    /// Withdraw idle balance only. This is meant for a provider to sweep their
    /// earned fees, but may also be used by seeker in case of any errors with
    /// full withdrawal flow.
    WithdrawIdleBalance {
        #[arg(value_enum)]
        account_type: AccountTypeArg,
        amount_msats: Amount,
    },
}
