use anyhow::Result;
use clap::{Args, Subcommand};
use polymarket_client_sdk_v2::bridge::{
    self,
    types::{DepositRequest, StatusRequest, WithdrawRequest},
};

use crate::output::OutputFormat;
use crate::output::bridge::{print_deposit, print_status, print_supported_assets, print_withdraw};

#[derive(Args)]
pub struct BridgeArgs {
    #[command(subcommand)]
    pub command: BridgeCommand,
}

#[derive(Subcommand)]
pub enum BridgeCommand {
    /// Get deposit addresses for a wallet (EVM, Solana, Bitcoin)
    Deposit {
        /// Polymarket wallet address (0x...), or `@`/`me` for your own wallet
        #[arg(value_parser = crate::auth::parse_address_or_me)]
        address: polymarket_client_sdk_v2::types::Address,
    },

    /// List supported chains and tokens for deposits
    SupportedAssets,

    /// Check deposit transaction status for an address
    Status {
        /// Deposit address (EVM, Solana, or Bitcoin), or `@`/`me` for your own wallet
        address: String,
    },

    /// Get a withdrawal address to move USDC.e out to another chain/token
    Withdraw {
        /// Destination chain ID (1=Ethereum, 137=Polygon, 8453=Base, ...)
        #[arg(long)]
        to_chain: u64,
        /// Destination token contract address (0x...)
        #[arg(long)]
        token: String,
        /// Recipient wallet address on the destination chain
        #[arg(long)]
        to: String,
        /// Source Polymarket wallet (defaults to your configured wallet; accepts `@`/`me`)
        #[arg(long, value_parser = crate::auth::parse_address_or_me)]
        address: Option<polymarket_client_sdk_v2::types::Address>,
    },
}

pub async fn execute(
    client: &bridge::Client,
    args: BridgeArgs,
    output: OutputFormat,
) -> Result<()> {
    match args.command {
        BridgeCommand::Deposit { address } => {
            let request = DepositRequest::builder().address(address).build();

            let response = client.deposit(&request).await?;
            print_deposit(&response, &output)?;
        }

        BridgeCommand::SupportedAssets => {
            let response = client.supported_assets().await?;
            print_supported_assets(&response, &output)?;
        }

        BridgeCommand::Status { address } => {
            anyhow::ensure!(!address.trim().is_empty(), "Address cannot be empty");
            let address = match address.trim() {
                "@" | "me" | "self" => crate::auth::my_address()?.to_string(),
                _ => address,
            };
            let request = StatusRequest::builder().address(&address).build();

            let response = client.status(&request).await?;
            print_status(&response, &output)?;
        }

        BridgeCommand::Withdraw {
            to_chain,
            token,
            to,
            address,
        } => {
            anyhow::ensure!(!token.trim().is_empty(), "Token address cannot be empty");
            anyhow::ensure!(!to.trim().is_empty(), "Recipient address cannot be empty");
            // Default the source to the configured wallet — that's where the USDC.e lives.
            let source = match address {
                Some(a) => a,
                None => {
                    let signer = crate::auth::resolve_signer(None)?;
                    polymarket_client_sdk_v2::auth::Signer::address(&signer)
                }
            };
            let request = WithdrawRequest::builder()
                .address(source)
                .to_chain_id(to_chain)
                .to_token_address(token)
                .recipient_addr(to)
                .build();

            let response = client.withdraw(&request).await?;
            print_withdraw(&response, &output)?;
        }
    }

    Ok(())
}

/// TUI-friendly deposit address lookup — returns the EVM deposit address string.
pub(crate) async fn tui_deposit_address() -> Result<String> {
    let client = bridge::Client::default();
    let address = {
        let signer = crate::auth::resolve_signer(None)?;
        polymarket_client_sdk_v2::auth::Signer::address(&signer)
    };
    let request = DepositRequest::builder().address(address).build();
    let response = client.deposit(&request).await?;
    Ok(format!("Deposit USDC.e to: {} (EVM)", response.address.evm))
}

/// TUI-friendly deposit status check — returns a one-line summary.
#[allow(dead_code)]
pub(crate) async fn tui_deposit_status() -> Result<String> {
    let client = bridge::Client::default();
    let address = {
        let signer = crate::auth::resolve_signer(None)?;
        polymarket_client_sdk_v2::auth::Signer::address(&signer)
    };
    let request = StatusRequest::builder()
        .address(address.to_string())
        .build();
    let response = client.status(&request).await?;
    let pending: Vec<_> = response
        .transactions
        .iter()
        .filter(|t| {
            !matches!(
                t.status,
                polymarket_client_sdk_v2::bridge::types::DepositTransactionStatus::Completed
            )
        })
        .collect();
    if pending.is_empty() {
        Ok("No pending deposits.".into())
    } else {
        Ok(format!(
            "{} pending deposit(s). Run `polymarket bridge status {}` for details.",
            pending.len(),
            address
        ))
    }
}
