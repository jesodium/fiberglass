use crate::dataapi::{
    Activity, BuilderEntry, LeaderboardEntry, LiveVolume, MetaHolder, OpenInterest, PortfolioValue,
    Position, UserStats,
};
use serde_json::json;
use tabled::settings::Style;
use tabled::{Table, Tabled};

use super::{DASH, OutputFormat, format_decimal, truncate};

fn format_market(condition_id: &str) -> String {
    if condition_id.eq_ignore_ascii_case("global") {
        "Global".into()
    } else {
        condition_id.into()
    }
}

pub fn print_pnl_summary(positions: &[Position], output: &OutputFormat) -> anyhow::Result<()> {
    use polymarket_client_sdk_v2::types::Decimal;
    let cost: Decimal = positions.iter().map(|p| p.entry_cost_usdc).sum();
    let value: Decimal = positions.iter().map(|p| p.current_value).sum();
    let unrealized: Decimal = positions.iter().map(|p| p.unrealized_pnl).sum();
    let realized: Decimal = positions.iter().map(|p| p.realized_pnl).sum();
    let total = unrealized + realized;
    // ROI on cost basis; zero cost => zero ROI rather than a divide-by-zero.
    let roi = if cost.is_zero() {
        Decimal::ZERO
    } else {
        total / cost * Decimal::from(100)
    };

    match output {
        OutputFormat::Table => {
            println!("Open positions:   {}", positions.len());
            println!("Cost basis:       {}", format_decimal(cost));
            println!("Current value:    {}", format_decimal(value));
            println!("Unrealized PnL:   {:+.2}", unrealized);
            println!("Realized PnL:     {:+.2}", realized);
            println!("Total PnL:        {:+.2}", total);
            println!("ROI:              {:+.2}%", roi);
        }
        OutputFormat::Json => {
            super::print_json(&json!({
                "open_positions": positions.len(),
                "cost_basis": cost.to_string(),
                "current_value": value.to_string(),
                "unrealized_pnl": unrealized.to_string(),
                "realized_pnl": realized.to_string(),
                "total_pnl": total.to_string(),
                "roi_pct": roi.to_string(),
            }))?;
        }
    }
    Ok(())
}

pub fn print_positions(positions: &[Position], output: &OutputFormat) -> anyhow::Result<()> {
    match output {
        OutputFormat::Table => {
            if positions.is_empty() {
                println!("No positions found.");
                return Ok(());
            }
            #[derive(Tabled)]
            struct Row {
                #[tabled(rename = "Market")]
                title: String,
                #[tabled(rename = "Outcome")]
                outcome: String,
                #[tabled(rename = "Size")]
                size: String,
                #[tabled(rename = "Avg Price")]
                avg_price: String,
                #[tabled(rename = "Current")]
                current_value: String,
                #[tabled(rename = "PnL")]
                pnl: String,
            }
            let rows: Vec<Row> = positions
                .iter()
                .map(|p| Row {
                    title: truncate(&p.title, 40),
                    outcome: p.outcome.clone(),
                    size: format!("{:.2}", p.current_size),
                    avg_price: format!("{:.4}", p.avg_price),
                    current_value: format_decimal(p.current_value),
                    pnl: format!("{:.2}", p.unrealized_pnl),
                })
                .collect();
            let table = Table::new(rows).with(Style::rounded()).to_string();
            println!("{table}");
        }
        OutputFormat::Json => {
            let data: Vec<_> = positions
                .iter()
                .map(|p| {
                    json!({
                        "title": p.title,
                        "slug": p.slug,
                        "icon": p.icon,
                        "event_slug": p.event_slug,
                        "event_id": p.event_id,
                        "outcome": p.outcome,
                        "outcome_index": p.outcome_index,
                        "opposite_outcome": p.opposite_outcome,
                        "opposite_asset": p.opposite_token_id,
                        "asset": p.token_id,
                        "size": p.current_size.to_string(),
                        "avg_price": p.avg_price.to_string(),
                        "initial_value": p.entry_cost_usdc.to_string(),
                        "current_value": p.current_value.to_string(),
                        "cash_pnl": p.unrealized_pnl.to_string(),
                        "percent_pnl": p.percent_pnl.to_string(),
                        "total_bought": p.total_size.to_string(),
                        "realized_pnl": p.realized_pnl.to_string(),
                        "percent_realized_pnl": p.percent_realized_pnl.to_string(),
                        "cur_price": p.current_price.to_string(),
                        "condition_id": p.condition_id,
                        "proxy_wallet": p.proxy_wallet,
                        "redeemable": p.redeemable,
                        "mergeable": p.mergeable,
                        "end_date": (!p.end_date.is_empty()).then_some(&p.end_date),
                        "negative_risk": p.negative_risk,
                    })
                })
                .collect();
            super::print_json(&data)?;
        }
    }
    Ok(())
}

pub fn print_closed_positions(positions: &[Position], output: &OutputFormat) -> anyhow::Result<()> {
    match output {
        OutputFormat::Table => {
            if positions.is_empty() {
                println!("No closed positions found.");
                return Ok(());
            }
            #[derive(Tabled)]
            struct Row {
                #[tabled(rename = "Market")]
                title: String,
                #[tabled(rename = "Outcome")]
                outcome: String,
                #[tabled(rename = "Avg Price")]
                avg_price: String,
                #[tabled(rename = "Realized PnL")]
                realized_pnl: String,
            }
            let rows: Vec<Row> = positions
                .iter()
                .map(|p| Row {
                    title: truncate(&p.title, 40),
                    outcome: p.outcome.clone(),
                    avg_price: format!("{:.4}", p.avg_price),
                    realized_pnl: format!("{:.2}", p.realized_pnl),
                })
                .collect();
            let table = Table::new(rows).with(Style::rounded()).to_string();
            println!("{table}");
        }
        OutputFormat::Json => {
            let data: Vec<_> = positions
                .iter()
                .map(|p| {
                    json!({
                        "title": p.title,
                        "slug": p.slug,
                        "icon": p.icon,
                        "event_slug": p.event_slug,
                        "outcome": p.outcome,
                        "outcome_index": p.outcome_index,
                        "opposite_outcome": p.opposite_outcome,
                        "opposite_asset": p.opposite_token_id,
                        "asset": p.token_id,
                        "avg_price": p.avg_price.to_string(),
                        "total_bought": p.total_size.to_string(),
                        "realized_pnl": p.realized_pnl.to_string(),
                        "cur_price": p.current_price.to_string(),
                        "condition_id": p.condition_id,
                        "proxy_wallet": p.proxy_wallet,
                        "timestamp": p.last_event_at,
                        "end_date": p.end_date,
                    })
                })
                .collect();
            super::print_json(&data)?;
        }
    }
    Ok(())
}

pub fn print_value(values: &[PortfolioValue], output: &OutputFormat) -> anyhow::Result<()> {
    match output {
        OutputFormat::Table => {
            if values.is_empty() {
                println!("No value data found.");
                return Ok(());
            }
            #[derive(Tabled)]
            struct Row {
                #[tabled(rename = "User")]
                user: String,
                #[tabled(rename = "Value")]
                value: String,
            }
            let rows: Vec<Row> = values
                .iter()
                .map(|v| Row {
                    user: truncate(&v.proxy_wallet, 14),
                    value: format_decimal(v.value),
                })
                .collect();
            let table = Table::new(rows).with(Style::rounded()).to_string();
            println!("{table}");
        }
        OutputFormat::Json => {
            let data: Vec<_> = values
                .iter()
                .map(|v| json!({"user": v.proxy_wallet, "value": v.value.to_string()}))
                .collect();
            super::print_json(&data)?;
        }
    }
    Ok(())
}

pub fn print_traded(t: &UserStats, output: &OutputFormat) -> anyhow::Result<()> {
    match output {
        OutputFormat::Table => println!("{}: {} markets traded", t.proxy_wallet, t.trades),
        OutputFormat::Json => {
            super::print_json(&json!({
                "user": t.proxy_wallet,
                "traded": t.trades,
            }))?;
        }
    }
    Ok(())
}

pub fn print_trades(trades: &[Activity], output: &OutputFormat) -> anyhow::Result<()> {
    match output {
        OutputFormat::Table => {
            if trades.is_empty() {
                println!("No trades found.");
                return Ok(());
            }
            #[derive(Tabled)]
            struct Row {
                #[tabled(rename = "Market")]
                title: String,
                #[tabled(rename = "Side")]
                side: String,
                #[tabled(rename = "Outcome")]
                outcome: String,
                #[tabled(rename = "Size")]
                size: String,
                #[tabled(rename = "Price")]
                price: String,
            }
            let rows: Vec<Row> = trades
                .iter()
                .map(|t| Row {
                    title: truncate(t.title.as_deref().unwrap_or(DASH), 40),
                    side: t.side.clone().unwrap_or_default(),
                    outcome: t.outcome.as_deref().unwrap_or(DASH).into(),
                    size: format!("{:.2}", t.size),
                    price: t.price.map(|p| format!("{:.4}", p)).unwrap_or_default(),
                })
                .collect();
            let table = Table::new(rows).with(Style::rounded()).to_string();
            println!("{table}");
        }
        OutputFormat::Json => {
            let data: Vec<_> = trades
                .iter()
                .map(|t| {
                    json!({
                        "title": t.title,
                        "slug": t.slug,
                        "icon": t.icon,
                        "event_slug": t.event_slug,
                        "side": t.side,
                        "outcome": t.outcome,
                        "outcome_index": t.outcome_index,
                        "asset": t.token_id,
                        "size": t.size.to_string(),
                        "price": t.price.map(|p| p.to_string()),
                        "timestamp": t.timestamp,
                        "condition_id": t.condition_id,
                        "proxy_wallet": t.proxy_wallet,
                        "transaction_hash": t.transaction_hash,
                        "name": t.name,
                        "pseudonym": t.pseudonym,
                        "bio": t.bio,
                        "profile_image": t.profile_image,
                        "profile_image_optimized": t.profile_image_optimized,
                    })
                })
                .collect();
            super::print_json(&data)?;
        }
    }
    Ok(())
}

pub fn print_activity(activity: &[Activity], output: &OutputFormat) -> anyhow::Result<()> {
    match output {
        OutputFormat::Table => {
            if activity.is_empty() {
                println!("No activity found.");
                return Ok(());
            }
            #[derive(Tabled)]
            struct Row {
                #[tabled(rename = "Type")]
                activity_type: String,
                #[tabled(rename = "Market")]
                title: String,
                #[tabled(rename = "Side")]
                side: String,
                #[tabled(rename = "Outcome")]
                outcome: String,
                #[tabled(rename = "Price")]
                price: String,
                #[tabled(rename = "Size")]
                size: String,
                #[tabled(rename = "Collateral")]
                usdc_size: String,
                #[tabled(rename = "Tx")]
                tx: String,
            }
            let rows: Vec<Row> = activity
                .iter()
                .map(|a| Row {
                    activity_type: a.kind.clone(),
                    title: truncate(a.title.as_deref().unwrap_or(DASH), 35),
                    side: a.side.clone().unwrap_or_default(),
                    outcome: a.outcome.as_deref().unwrap_or(DASH).into(),
                    price: a.price.map(|p| format!("{:.4}", p)).unwrap_or_default(),
                    size: format!("{:.2}", a.size),
                    usdc_size: format_decimal(a.usdc_size),
                    tx: truncate(&a.transaction_hash, 14),
                })
                .collect();
            let table = Table::new(rows).with(Style::rounded()).to_string();
            println!("{table}");
        }
        OutputFormat::Json => {
            let data: Vec<_> = activity
                .iter()
                .map(|a| {
                    json!({
                        "activity_type": a.kind,
                        "title": a.title,
                        "slug": a.slug,
                        "icon": a.icon,
                        "event_slug": a.event_slug,
                        "side": a.side,
                        "outcome": a.outcome,
                        "outcome_index": a.outcome_index,
                        "price": a.price.map(|p| p.to_string()),
                        "asset": a.token_id,
                        "condition_id": a.condition_id,
                        "size": a.size.to_string(),
                        "usdc_size": a.usdc_size.to_string(),
                        "timestamp": a.timestamp,
                        "transaction_hash": a.transaction_hash,
                        "proxy_wallet": a.proxy_wallet,
                        "name": a.name,
                        "pseudonym": a.pseudonym,
                        "bio": a.bio,
                        "profile_image": a.profile_image,
                        "profile_image_optimized": a.profile_image_optimized,
                    })
                })
                .collect();
            super::print_json(&data)?;
        }
    }
    Ok(())
}

pub fn print_holders(meta_holders: &[MetaHolder], output: &OutputFormat) -> anyhow::Result<()> {
    match output {
        OutputFormat::Table => {
            if meta_holders.is_empty() {
                println!("No holders found.");
                return Ok(());
            }
            #[derive(Tabled)]
            struct Row {
                #[tabled(rename = "Wallet")]
                wallet: String,
                #[tabled(rename = "Name")]
                name: String,
                #[tabled(rename = "Amount")]
                amount: String,
                #[tabled(rename = "Outcome")]
                outcome_index: String,
            }
            let rows: Vec<Row> = meta_holders
                .iter()
                .flat_map(|mh| {
                    mh.holders.iter().map(|h| Row {
                        wallet: truncate(&h.proxy_wallet, 14),
                        name: h
                            .name
                            .as_deref()
                            .or(h.pseudonym.as_deref())
                            .unwrap_or(DASH)
                            .into(),
                        amount: format_decimal(h.amount),
                        outcome_index: h.outcome_index.to_string(),
                    })
                })
                .collect();
            let table = Table::new(rows).with(Style::rounded()).to_string();
            println!("{table}");
        }
        OutputFormat::Json => {
            let data: Vec<_> = meta_holders
                .iter()
                .map(|mh| {
                    let holders: Vec<_> = mh
                        .holders
                        .iter()
                        .map(|h| {
                            json!({
                                "proxy_wallet": h.proxy_wallet,
                                "name": h.name,
                                "pseudonym": h.pseudonym,
                                "bio": h.bio,
                                "asset": h.token_id,
                                "amount": h.amount.to_string(),
                                "outcome_index": h.outcome_index,
                                "display_username_public": h.display_username_public,
                                "profile_image": h.profile_image,
                                "profile_image_optimized": h.profile_image_optimized,
                                "verified": h.verified,
                            })
                        })
                        .collect();
                    json!({"token": mh.token_id, "holders": holders})
                })
                .collect();
            super::print_json(&data)?;
        }
    }
    Ok(())
}

pub fn print_open_interest(oi: &[OpenInterest], output: &OutputFormat) -> anyhow::Result<()> {
    match output {
        OutputFormat::Table => {
            if oi.is_empty() {
                println!("No open interest data found.");
                return Ok(());
            }
            #[derive(Tabled)]
            struct Row {
                #[tabled(rename = "Market")]
                market: String,
                #[tabled(rename = "Open Interest")]
                value: String,
            }
            let rows: Vec<Row> = oi
                .iter()
                .map(|o| Row {
                    market: format_market(&o.condition_id),
                    value: format_decimal(o.value),
                })
                .collect();
            let table = Table::new(rows).with(Style::rounded()).to_string();
            println!("{table}");
        }
        OutputFormat::Json => {
            let data: Vec<_> = oi
                .iter()
                .map(|o| json!({"market": format_market(&o.condition_id), "value": o.value.to_string()}))
                .collect();
            super::print_json(&data)?;
        }
    }
    Ok(())
}

pub fn print_live_volume(v: &LiveVolume, output: &OutputFormat) -> anyhow::Result<()> {
    match output {
        OutputFormat::Table => {
            #[derive(Tabled)]
            struct Row {
                #[tabled(rename = "Market")]
                market: String,
                #[tabled(rename = "Volume")]
                value: String,
            }
            println!("Total: {}", format_decimal(v.taker_volume_total));
            let rows: Vec<Row> = v
                .conditions
                .iter()
                .map(|c| Row {
                    market: format_market(&c.condition_id),
                    value: format_decimal(c.taker_volume),
                })
                .collect();
            let table = Table::new(rows).with(Style::rounded()).to_string();
            println!("{table}");
        }
        OutputFormat::Json => {
            let markets: Vec<_> = v
                .conditions
                .iter()
                .map(|c| json!({"market": format_market(&c.condition_id), "value": c.taker_volume.to_string()}))
                .collect();
            // Array of one, matching the v1 output shape.
            super::print_json(
                &json!([{"total": v.taker_volume_total.to_string(), "markets": markets}]),
            )?;
        }
    }
    Ok(())
}

pub fn print_leaderboard(
    entries: &[LeaderboardEntry],
    output: &OutputFormat,
) -> anyhow::Result<()> {
    match output {
        OutputFormat::Table => {
            if entries.is_empty() {
                println!("No leaderboard entries found.");
                return Ok(());
            }
            #[derive(Tabled)]
            struct Row {
                #[tabled(rename = "#")]
                rank: String,
                #[tabled(rename = "Trader")]
                trader: String,
                #[tabled(rename = "PnL")]
                pnl: String,
                #[tabled(rename = "Volume")]
                volume: String,
            }
            let rows: Vec<Row> = entries
                .iter()
                .map(|e| Row {
                    rank: e.rank.to_string(),
                    trader: truncate(e.user_name.as_deref().unwrap_or(DASH), 20),
                    pnl: format_decimal(e.pnl),
                    volume: format_decimal(e.volume),
                })
                .collect();
            let table = Table::new(rows).with(Style::rounded()).to_string();
            println!("{table}");
        }
        OutputFormat::Json => {
            let data: Vec<_> = entries
                .iter()
                .map(|e| {
                    json!({
                        "rank": e.rank,
                        "proxy_wallet": e.user_id,
                        "user_name": e.user_name,
                        "pnl": e.pnl.to_string(),
                        "volume": e.volume.to_string(),
                        "profile_image": e.profile_image,
                        "x_username": e.x_username,
                        "verified_badge": e.verified,
                    })
                })
                .collect();
            super::print_json(&data)?;
        }
    }
    Ok(())
}

pub fn print_builder_leaderboard(
    entries: &[BuilderEntry],
    output: &OutputFormat,
) -> anyhow::Result<()> {
    match output {
        OutputFormat::Table => {
            if entries.is_empty() {
                println!("No builder leaderboard entries found.");
                return Ok(());
            }
            #[derive(Tabled)]
            struct Row {
                #[tabled(rename = "#")]
                rank: String,
                #[tabled(rename = "Builder")]
                builder: String,
                #[tabled(rename = "Volume")]
                volume: String,
                #[tabled(rename = "Users")]
                active_users: String,
            }
            let rows: Vec<Row> = entries
                .iter()
                .map(|e| Row {
                    rank: e.rank.to_string(),
                    builder: truncate(&e.builder_name, 25),
                    volume: format_decimal(e.volume),
                    active_users: e.active_users.to_string(),
                })
                .collect();
            let table = Table::new(rows).with(Style::rounded()).to_string();
            println!("{table}");
        }
        OutputFormat::Json => {
            let data: Vec<_> = entries
                .iter()
                .map(|e| {
                    json!({
                        "rank": e.rank,
                        "builder": e.builder_name,
                        "volume": e.volume.to_string(),
                        "active_users": e.active_users,
                        "verified": e.verified,
                        "builder_logo": e.profile_image,
                    })
                })
                .collect();
            super::print_json(&data)?;
        }
    }
    Ok(())
}

pub fn print_builder_volume(entries: &[BuilderEntry], output: &OutputFormat) -> anyhow::Result<()> {
    match output {
        OutputFormat::Table => {
            if entries.is_empty() {
                println!("No builder volume data found.");
                return Ok(());
            }
            #[derive(Tabled)]
            struct Row {
                #[tabled(rename = "Date")]
                date: String,
                #[tabled(rename = "Builder")]
                builder: String,
                #[tabled(rename = "Volume")]
                volume: String,
                #[tabled(rename = "Users")]
                active_users: String,
                #[tabled(rename = "#")]
                rank: String,
            }
            let rows: Vec<Row> = entries
                .iter()
                .map(|e| Row {
                    date: e.date.clone(),
                    builder: truncate(&e.builder_name, 25),
                    volume: format_decimal(e.volume),
                    active_users: e.active_users.to_string(),
                    rank: e.rank.to_string(),
                })
                .collect();
            let table = Table::new(rows).with(Style::rounded()).to_string();
            println!("{table}");
        }
        OutputFormat::Json => {
            let data: Vec<_> = entries
                .iter()
                .map(|e| {
                    json!({
                        "date": e.date,
                        "builder": e.builder_name,
                        "builder_logo": e.profile_image,
                        "volume": e.volume.to_string(),
                        "active_users": e.active_users,
                        "rank": e.rank,
                        "verified": e.verified,
                    })
                })
                .collect();
            super::print_json(&data)?;
        }
    }
    Ok(())
}
