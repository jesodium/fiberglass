use anyhow::Result;
use clap::{Args, Subcommand};
use polymarket_client_sdk_v2::types::{Address, B256};

use crate::dataapi::{self, MAX_PAGE};
use crate::output::OutputFormat;
use crate::output::data::{
    print_activity, print_builder_leaderboard, print_builder_volume, print_closed_positions,
    print_holders, print_leaderboard, print_live_volume, print_open_interest, print_pnl_summary,
    print_positions, print_traded, print_trades, print_value,
};

#[derive(Args)]
pub struct DataArgs {
    #[command(subcommand)]
    pub command: DataCommand,
}

#[derive(Subcommand)]
pub enum DataCommand {
    /// Get open positions for a wallet address
    Positions {
        /// Wallet address (0x...), or `@`/`me` for your own wallet
        #[arg(value_parser = crate::auth::parse_address_or_me)]
        address: Address,

        /// Max results
        #[arg(long, default_value = "25")]
        limit: i32,

        /// Pagination offset
        #[arg(long)]
        offset: Option<i32>,
    },

    /// Aggregate PnL summary across all open positions for a wallet
    Pnl {
        /// Wallet address (0x...), or `@`/`me` for your own wallet
        #[arg(value_parser = crate::auth::parse_address_or_me)]
        address: Address,
    },

    /// Get closed positions for a wallet address
    ClosedPositions {
        /// Wallet address (0x...), or `@`/`me` for your own wallet
        #[arg(value_parser = crate::auth::parse_address_or_me)]
        address: Address,

        /// Max results
        #[arg(long, default_value = "25")]
        limit: i32,

        /// Pagination offset
        #[arg(long)]
        offset: Option<i32>,
    },

    /// Get total position value for a wallet address
    Value {
        /// Wallet address (0x...), or `@`/`me` for your own wallet
        #[arg(value_parser = crate::auth::parse_address_or_me)]
        address: Address,
    },

    /// Get count of unique markets traded by a wallet
    Traded {
        /// Wallet address (0x...), or `@`/`me` for your own wallet
        #[arg(value_parser = crate::auth::parse_address_or_me)]
        address: Address,
    },

    /// Get trade history
    Trades {
        /// Wallet address (0x...), or `@`/`me` for your own wallet
        #[arg(value_parser = crate::auth::parse_address_or_me)]
        address: Address,

        /// Max results
        #[arg(long, default_value = "25")]
        limit: i32,

        /// Pagination offset
        #[arg(long)]
        offset: Option<i32>,
    },

    /// Get on-chain activity for a wallet address
    Activity {
        /// Wallet address (0x...), or `@`/`me` for your own wallet
        #[arg(value_parser = crate::auth::parse_address_or_me)]
        address: Address,

        /// Max results
        #[arg(long, default_value = "25")]
        limit: i32,

        /// Pagination offset
        #[arg(long)]
        offset: Option<i32>,
    },

    /// Get top token holders for a market
    Holders {
        /// Market condition ID (0x...)
        market: B256,

        /// Max results per token
        #[arg(long, default_value = "10")]
        limit: i32,
    },

    /// Get open interest for markets
    OpenInterest {
        /// Market condition ID (0x...)
        market: B256,
    },

    /// Get live volume for an event
    Volume {
        /// Event ID
        id: u64,
    },

    /// Trader leaderboard
    Leaderboard {
        /// Time period: day, week, month, all
        #[arg(long)]
        period: Option<TimePeriod>,

        /// Order by: pnl or vol
        #[arg(long)]
        order_by: Option<OrderBy>,

        /// Max results
        #[arg(long, default_value = "25")]
        limit: i32,

        /// Pagination offset
        #[arg(long)]
        offset: Option<i32>,
    },

    /// Builder leaderboard
    BuilderLeaderboard {
        /// Time period: day, week, month, all
        #[arg(long)]
        period: Option<TimePeriod>,

        /// Max results
        #[arg(long, default_value = "25")]
        limit: i32,

        /// Pagination offset
        #[arg(long)]
        offset: Option<i32>,
    },

    /// Builder volume time-series
    BuilderVolume {
        /// Time period: day, week, month, all
        #[arg(long)]
        period: Option<TimePeriod>,
    },
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum TimePeriod {
    Day,
    Week,
    Month,
    All,
}

impl TimePeriod {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::All => "all",
        }
    }
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum OrderBy {
    Pnl,
    Vol,
}

/// v1 `--limit`/`--offset` flags (kept for CLI/MCP compatibility) as a v2 window.
fn window(limit: i32, offset: Option<i32>) -> (usize, usize) {
    (
        usize::try_from(limit).unwrap_or(0).max(1),
        offset.and_then(|o| usize::try_from(o).ok()).unwrap_or(0),
    )
}

pub async fn execute(args: DataArgs, output: OutputFormat) -> Result<()> {
    match args.command {
        DataCommand::Positions {
            address,
            limit,
            offset,
        } => {
            let (limit, offset) = window(limit, offset);
            let q = vec![("user", address.to_string())];
            let positions = dataapi::list_window("positions", q, limit, offset).await?;
            print_positions(&positions, &output)?;
        }

        DataCommand::Pnl { address } => {
            // IMPORTANT NOTE: one max-size page (1000 positions); raise the cap if anyone holds more.
            let q = vec![("user", address.to_string())];
            let positions = dataapi::list("positions", q, MAX_PAGE).await?;
            print_pnl_summary(&positions, &output)?;
        }

        DataCommand::ClosedPositions {
            address,
            limit,
            offset,
        } => {
            let (limit, offset) = window(limit, offset);
            let q = vec![("user", address.to_string()), ("status", "CLOSED".into())];
            let positions = dataapi::list_window("positions", q, limit, offset).await?;
            print_closed_positions(&positions, &output)?;
        }

        DataCommand::Value { address } => {
            let value = dataapi::get("value", vec![("user", address.to_string())]).await?;
            print_value(&[value], &output)?;
        }

        DataCommand::Traded { address } => {
            let stats = dataapi::get("user-stats", vec![("user", address.to_string())]).await?;
            print_traded(&stats, &output)?;
        }

        DataCommand::Trades {
            address,
            limit,
            offset,
        } => {
            let (limit, offset) = window(limit, offset);
            // v2 defaults to taker-only; a wallet's history should include its maker fills.
            let q = vec![
                ("user", address.to_string()),
                ("taker_only", "false".into()),
            ];
            let trades = dataapi::list_window("trades", q, limit, offset).await?;
            print_trades(&trades, &output)?;
        }

        DataCommand::Activity {
            address,
            limit,
            offset,
        } => {
            let (limit, offset) = window(limit, offset);
            let q = vec![("user", address.to_string())];
            let activity = dataapi::list_window("activity", q, limit, offset).await?;
            print_activity(&activity, &output)?;
        }

        DataCommand::Holders { market, limit } => {
            // `limit` is per outcome token here, and the payload is grouped by token.
            let q = vec![
                ("condition", market.to_string()),
                ("limit", limit.to_string()),
            ];
            let holders: Vec<dataapi::MetaHolder> = dataapi::get("holders", q).await?;
            print_holders(&holders, &output)?;
        }

        DataCommand::OpenInterest { market } => {
            let oi: Vec<dataapi::OpenInterest> =
                dataapi::get("oi", vec![("condition", market.to_string())]).await?;
            print_open_interest(&oi, &output)?;
        }

        DataCommand::Volume { id } => {
            let volume = dataapi::get("live-volume", vec![("event_id", id.to_string())]).await?;
            print_live_volume(&volume, &output)?;
        }

        DataCommand::Leaderboard {
            period,
            order_by,
            limit,
            offset,
        } => {
            let (limit, offset) = window(limit, offset);
            let mut q = Vec::new();
            if let Some(p) = period {
                q.push(("time_period", p.as_str().to_string()));
            }
            if let Some(o) = order_by {
                let board = match o {
                    OrderBy::Pnl => "PNL",
                    OrderBy::Vol => "VOLUME",
                };
                q.push(("sort_by", board.to_string()));
            }
            let entries = dataapi::list_window("leaderboard", q, limit, offset).await?;
            print_leaderboard(&entries, &output)?;
        }

        DataCommand::BuilderLeaderboard {
            period,
            limit,
            offset,
        } => {
            let (limit, offset) = window(limit, offset);
            let q = period
                .map(|p| ("time_period", p.as_str().to_string()))
                .into_iter()
                .collect();
            let entries = dataapi::list_window("builders/leaderboard", q, limit, offset).await?;
            print_builder_leaderboard(&entries, &output)?;
        }

        DataCommand::BuilderVolume { period } => {
            let q = period
                .map(|p| ("interval", p.as_str().to_string()))
                .into_iter()
                .collect();
            let entries: Vec<dataapi::BuilderEntry> = dataapi::get("builders/volume", q).await?;
            print_builder_volume(&entries, &output)?;
        }
    }

    Ok(())
}
