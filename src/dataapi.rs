//! Polymarket Data API v2 client (`data-api.polymarket.com/v2`).
//!
//! v1 was retired (every path answers `410 Gone`) and the SDK's data client still
//! targets v1, so the CLI talks to v2 directly. Every v2 route wraps its payload
//! in `{data, pagination?}`; list routes page by opaque cursor only (no offset).
//! Only the fields the CLI/TUI/copy-trade engine read are modelled here.

use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use polymarket_client_sdk_v2::types::Decimal;
use serde::Deserialize;
use serde::de::DeserializeOwned;

const BASE: &str = "https://data-api.polymarket.com/v2";
/// Server-side cap on `limit` for every paged route.
pub const MAX_PAGE: usize = 1000;

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

pub type Query<'a> = Vec<(&'a str, String)>;

#[derive(Deserialize)]
struct Envelope<T> {
    data: T,
    #[serde(default)]
    pagination: Option<Pagination>,
}

#[derive(Deserialize)]
struct Pagination {
    #[serde(default)]
    has_more: bool,
    next_cursor: Option<String>,
}

async fn fetch<T: DeserializeOwned>(route: &str, query: &[(&str, String)]) -> Result<Envelope<T>> {
    let resp = HTTP
        .get(format!("{BASE}/{route}"))
        .query(query)
        .send()
        .await
        .with_context(|| format!("data-api /v2/{route} request failed"))?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        // v2 errors are `{"error": "...", "code": "...", ...}`; surface the message.
        let msg = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_owned))
            .unwrap_or(body);
        bail!("data-api /v2/{route}: {status}: {msg}");
    }
    serde_json::from_str(&body)
        .with_context(|| format!("data-api /v2/{route}: unexpected response"))
}

/// Single-object (non-paged) route: returns `data`.
pub async fn get<T: DeserializeOwned>(route: &str, query: Query<'_>) -> Result<T> {
    Ok(fetch(route, &query).await?.data)
}

/// Paged route: follows `next_cursor` until `max` rows are collected or the
/// feed ends. A cursor carries its own page size, so `limit` is sent only on
/// the first request.
pub async fn list<T: DeserializeOwned>(
    route: &str,
    mut query: Query<'_>,
    max: usize,
) -> Result<Vec<T>> {
    query.push(("limit", max.clamp(1, MAX_PAGE).to_string()));
    let mut out: Vec<T> = Vec::new();
    loop {
        let page: Envelope<Vec<T>> = fetch(route, &query).await?;
        out.extend(page.data);
        let next = page
            .pagination
            .filter(|p| p.has_more)
            .and_then(|p| p.next_cursor);
        match next {
            Some(cursor) if out.len() < max => {
                query.retain(|(k, _)| *k != "limit" && *k != "cursor");
                query.push(("cursor", cursor));
            }
            _ => break,
        }
    }
    out.truncate(max);
    Ok(out)
}

/// Emulates v1 `offset`/`limit` paging on the cursor-only v2 feeds: fetch
/// `offset + limit` rows and drop the first `offset`.
pub async fn list_window<T: DeserializeOwned>(
    route: &str,
    query: Query<'_>,
    limit: usize,
    offset: usize,
) -> Result<Vec<T>> {
    let mut rows = list(route, query, offset + limit).await?;
    Ok(rows.split_off(offset.min(rows.len())))
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Position {
    pub proxy_wallet: String,
    pub token_id: String,
    pub condition_id: String,
    pub current_size: Decimal,
    pub avg_price: Decimal,
    pub entry_cost_usdc: Decimal,
    pub current_price: Decimal,
    pub current_value: Decimal,
    pub total_size: Decimal,
    pub realized_pnl: Decimal,
    pub unrealized_pnl: Decimal,
    pub percent_pnl: Decimal,
    pub percent_realized_pnl: Decimal,
    pub redeemable: bool,
    pub mergeable: bool,
    pub negative_risk: bool,
    pub title: String,
    pub slug: String,
    pub icon: String,
    pub event_id: String,
    pub event_slug: String,
    pub outcome: String,
    pub outcome_index: i64,
    pub opposite_outcome: String,
    pub opposite_token_id: String,
    pub end_date: String,
    pub last_event_at: i64,
}

/// Row of `/v2/trades` and `/v2/activity` (activity adds `type` + `usdc_size`;
/// non-trade activity may omit side/price/token).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Activity {
    #[serde(rename = "type")]
    pub kind: String,
    pub proxy_wallet: String,
    pub timestamp: i64,
    pub condition_id: Option<String>,
    pub token_id: Option<String>,
    pub side: Option<String>,
    pub price: Option<Decimal>,
    pub size: Decimal,
    pub usdc_size: Decimal,
    pub transaction_hash: String,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub icon: Option<String>,
    pub event_slug: Option<String>,
    pub outcome: Option<String>,
    pub outcome_index: Option<i64>,
    pub name: Option<String>,
    pub pseudonym: Option<String>,
    pub bio: Option<String>,
    pub profile_image: Option<String>,
    pub profile_image_optimized: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Holder {
    pub proxy_wallet: String,
    pub token_id: String,
    pub amount: Decimal,
    pub outcome_index: i64,
    pub name: Option<String>,
    pub pseudonym: Option<String>,
    pub bio: Option<String>,
    pub display_username_public: bool,
    pub profile_image: Option<String>,
    pub profile_image_optimized: Option<String>,
    pub verified: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MetaHolder {
    pub token_id: String,
    pub holders: Vec<Holder>,
}

/// `/v2/oi` row; `condition_id` is `"GLOBAL"` when no condition was given.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OpenInterest {
    pub condition_id: String,
    pub value: Decimal,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LiveVolume {
    pub taker_volume_total: Decimal,
    pub conditions: Vec<ConditionVolume>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ConditionVolume {
    pub condition_id: String,
    pub taker_volume: Decimal,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PortfolioValue {
    pub proxy_wallet: String,
    pub value: Decimal,
}

/// `/v2/user-stats`; `trades` is the count of markets traded (v1 `/traded`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UserStats {
    pub proxy_wallet: String,
    pub trades: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LeaderboardEntry {
    pub rank: i64,
    pub user_id: String,
    pub user_name: Option<String>,
    pub pnl: Decimal,
    pub volume: Decimal,
    pub profile_image: Option<String>,
    pub x_username: Option<String>,
    pub verified: bool,
}

/// Row of `/v2/builders/leaderboard` and `/v2/builders/volume` (`date` only on the latter).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct BuilderEntry {
    pub date: String,
    pub rank: i64,
    pub builder_name: String,
    pub profile_image: Option<String>,
    pub verified: bool,
    pub volume: Decimal,
    pub active_users: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_v2_envelope_and_float_decimals() {
        let body = r#"{"data":[{"type":"TRADE","timestamp":1795005296,"side":"BUY","price":0.761,
            "size":24.0,"usdc_size":18.48225,"token_id":"8466","transaction_hash":"0x14"}],
            "pagination":{"limit":1,"offset":0,"has_more":true,"next_cursor":"abc"}}"#;
        let env: Envelope<Vec<Activity>> = serde_json::from_str(body).unwrap();
        let a = &env.data[0];
        assert_eq!(a.kind, "TRADE");
        assert_eq!(a.price, Some(Decimal::new(761, 3)));
        assert_eq!(a.usdc_size, Decimal::new(1848225, 5));
        assert_eq!(a.token_id.as_deref(), Some("8466"));
        let p = env.pagination.unwrap();
        assert!(p.has_more);
        assert_eq!(p.next_cursor.as_deref(), Some("abc"));
        // Non-paged routes carry no pagination block.
        let v: Envelope<PortfolioValue> =
            serde_json::from_str(r#"{"data":{"proxy_wallet":"0xad","value":476634.1477}}"#)
                .unwrap();
        assert!(v.pagination.is_none());
        assert_eq!(v.data.value, Decimal::new(4766341477, 4));
    }
}
