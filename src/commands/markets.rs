use anyhow::Result;
use clap::{Args, Subcommand};
use polymarket_client_sdk_v2::gamma::{
    self,
    types::{
        request::{
            MarketByIdRequest, MarketBySlugRequest, MarketTagsRequest, MarketsRequest,
            SearchRequest,
        },
        response::Market,
    },
};

use super::is_numeric_id;
use crate::output::OutputFormat;
use crate::output::markets::{print_market, print_markets};
use crate::output::tags::print_tags;

#[derive(Args)]
pub struct MarketsArgs {
    #[command(subcommand)]
    pub command: MarketsCommand,
}

#[derive(Subcommand)]
pub enum MarketsCommand {
    /// List markets with optional filters
    List {
        /// Filter by active status
        #[arg(long)]
        active: Option<bool>,

        /// Filter by closed status
        #[arg(long)]
        closed: Option<bool>,

        /// Max results
        #[arg(long, default_value = "25")]
        limit: i32,

        /// Pagination offset
        #[arg(long)]
        offset: Option<i32>,

        /// Sort field: `volumeNum`, `liquidityNum`, `volume24hr`, `startDate`.
        /// Note: keys are camelCase; unknown keys are silently ignored by the API
        /// (results come back unsorted).
        #[arg(long)]
        order: Option<String>,

        /// Sort ascending. Default is descending (highest first).
        #[arg(long)]
        ascending: bool,
    },

    /// Get a single market by ID or slug
    Get {
        /// Market ID (numeric) or slug
        id: String,
    },

    /// Search markets
    Search {
        /// Search query string
        query: String,

        /// Results per type
        #[arg(long, default_value = "10")]
        limit: i32,
    },

    /// Get tags for a market
    Tags {
        /// Market ID
        id: String,
    },
}

pub async fn execute(
    client: &gamma::Client,
    args: MarketsArgs,
    output: OutputFormat,
) -> Result<()> {
    match args.command {
        MarketsCommand::List {
            active,
            closed,
            limit,
            offset,
            order,
            ascending,
        } => {
            // Gamma's `/markets` defaults `closed` to true when the param is
            // omitted (contrary to its docs), so a flagless `markets list`
            // would return only settled markets. Default to open markets unless
            // the user explicitly narrows it via --closed/--active.
            let resolved_closed = closed.or_else(|| active.map(|a| !a)).or(Some(false));

            let request = MarketsRequest::builder()
                .limit(limit)
                .maybe_closed(resolved_closed)
                .maybe_offset(offset)
                .maybe_order(order)
                .ascending(ascending)
                .build();

            let markets = client.markets(&request).await?;
            print_markets(&markets, &output)?;
        }

        MarketsCommand::Get { id } => {
            let is_numeric = is_numeric_id(&id);
            let market = if is_numeric {
                let req = MarketByIdRequest::builder().id(id).build();
                client.market_by_id(&req).await?
            } else {
                let req = MarketBySlugRequest::builder().slug(id).build();
                client.market_by_slug(&req).await?
            };

            print_market(&market, &output)?;
        }

        MarketsCommand::Search { query, limit } => {
            let request = SearchRequest::builder()
                .q(query)
                .limit_per_type(limit)
                .build();

            let results = client.search(&request).await?;

            let markets: Vec<Market> = results
                .events
                .unwrap_or_default()
                .into_iter()
                .flat_map(|e| e.markets.unwrap_or_default())
                .collect();

            print_markets(&markets, &output)?;
        }

        MarketsCommand::Tags { id } => {
            let req = MarketTagsRequest::builder().id(id).build();
            let tags = client.market_tags(&req).await?;

            print_tags(&tags, &output)?;
        }
    }

    Ok(())
}
