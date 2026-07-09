//! TUI application state and input handling.

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use polymarket_client_sdk_v2::auth::{LocalSigner, Signer as _};
use polymarket_client_sdk_v2::types::Decimal;
use polymarket_client_sdk_v2::{POLYGON, derive_proxy_wallet};

use super::data::{MarketRow, ResolutionInfo, Shared};
use super::live::{LiveOpenOrder, LiveOrder, WalletInfo};
use crate::config;
use crate::copytrade::config::CopyTrader;
use crate::copytrade::engine::CopyEngine;
use crate::paper::engine as paper_engine;
use crate::paper::store;
use crate::paper::types::{
    MarketMeta, OpenOrder, OrderKind, PaperAccount, Position, PositionView, Quote, TradeSide,
    default_starting_balance,
};
use crate::settings::{self, Settings};

/// The screens of the terminal, in tab order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum View {
    Onboarding,
    Dashboard,
    Markets,
    MarketDetail,
    Portfolio,
    Positions,
    Orders,
    History,
    Copytrade,
    Logs,
    Settings,
}

impl View {
    /// Tabs shown in the top bar (MarketDetail is reached from Markets, not a
    /// top-level tab).
    pub const TABS: [View; 9] = [
        View::Dashboard,
        View::Markets,
        View::Portfolio,
        View::Positions,
        View::Orders,
        View::History,
        View::Copytrade,
        View::Logs,
        View::Settings,
    ];

    pub fn title(self) -> &'static str {
        match self {
            View::Onboarding => "Onboarding",
            View::Dashboard => "Dashboard",
            View::Markets => "Markets",
            View::MarketDetail => "Market",
            View::Portfolio => "Portfolio",
            View::Positions => "Positions",
            View::Orders => "Orders",
            View::History => "History",
            View::Copytrade => "Copytrade",
            View::Logs => "Logs",
            View::Settings => "Settings",
        }
    }
}

/// Modal order-entry form.
pub(crate) struct OrderModal {
    pub token_id: String,
    pub question: String,
    pub outcome: String,
    pub side: TradeSide,
    pub kind: OrderKind,
    /// Market: pUSD (buy) or shares (sell). Limit: ignored.
    pub amount: String,
    pub price: String,
    pub size: String,
    /// Take-profit percent (buys only); blank = none.
    pub tp: String,
    /// Stop-loss percent (buys only); blank = none.
    pub sl: String,
    pub field: ModalField,
    pub error: Option<String>,
    /// Index into the relevant preset list for the `p` quick-fill cycle.
    pub preset_idx: usize,
    /// Shares currently held in this token (for quicksell % presets).
    pub held: Decimal,
    /// True once the trading-mode confirmation gate has been shown and the
    /// next Enter should actually send the order.
    pub awaiting_confirm: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModalField {
    Amount,
    Price,
    Size,
    TakeProfit,
    StopLoss,
}

/// Which setting the inline editor is changing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingField {
    Threshold,
    Quickbuy,
    Quicksell,
    Slippage,
    TakeProfit,
    StopLoss,
    Trailing,
    CopyPoll,
}

impl SettingField {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Threshold => "Confirmation threshold ($)",
            Self::Quickbuy => "Quickbuy presets ($, comma list)",
            Self::Quicksell => "Quicksell presets (%, comma list)",
            Self::Slippage => "Slippage tolerance (%)",
            Self::TakeProfit => "Default take-profit (%, blank=off)",
            Self::StopLoss => "Default stop-loss (%, blank=off)",
            Self::Trailing => "Default trailing-stop (%, blank=off)",
            Self::CopyPoll => "Copy-trade poll (seconds)",
        }
    }
}

/// Inline editor for a single setting value.
pub(crate) struct SettingsEditModal {
    pub field: SettingField,
    pub input: String,
    pub error: Option<String>,
}

/// A row on the Settings tab — either the trading-mode toggle or an editable
/// value. The order here is the on-screen order and the selection index.
#[derive(Clone, Copy)]
pub(crate) enum SettingRow {
    Mode,
    /// Toggle: settle resolved markets automatically vs manual claim.
    AutoSettle,
    /// Toggle: start the background guard worker at login (macOS LaunchAgent).
    GuardAutostart,
    Field(SettingField),
}

/// Number of sortable columns in the Holdings table (Market, Outcome, Shares,
/// Avg, Mark, Value, uPnL). Used to bound the `o` sort-cycle key.
pub(crate) const HOLDINGS_SORT_COLS: usize = 7;
/// Sortable columns in the Positions table: Market, Out, Shares, Avg, Mark,
/// uPnL, ROI.
pub(crate) const POSITIONS_SORT_COLS: usize = 7;
/// Sortable columns in the Orders table (paper and live both expose 7).
pub(crate) const ORDERS_SORT_COLS: usize = 7;

fn cmp_opt<T: Ord>(a: Option<T>, b: Option<T>) -> std::cmp::Ordering {
    match (a, b) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

fn apply_dir(ord: std::cmp::Ordering, asc: bool) -> std::cmp::Ordering {
    if asc { ord } else { ord.reverse() }
}

fn side_rank(s: TradeSide) -> u8 {
    match s {
        TradeSide::Buy => 0,
        TradeSide::Sell => 1,
    }
}

/// Sort positions by the Positions-table column index: 0 Market, 1 Outcome,
/// 2 Shares, 3 Avg, 4 Mark, 5 uPnL, 6 ROI.
fn sort_position_views(list: &mut [PositionView], col: usize, asc: bool) {
    list.sort_by(|a, b| {
        let ord = match col {
            0 => a
                .position
                .question
                .to_lowercase()
                .cmp(&b.position.question.to_lowercase()),
            1 => a
                .position
                .outcome
                .to_lowercase()
                .cmp(&b.position.outcome.to_lowercase()),
            2 => a.position.size.cmp(&b.position.size),
            3 => a.position.avg_price.cmp(&b.position.avg_price),
            4 => cmp_opt(a.mark_price, b.mark_price),
            5 => cmp_opt(a.unrealized_pnl, b.unrealized_pnl),
            6 => cmp_opt(a.roi(), b.roi()),
            _ => std::cmp::Ordering::Equal,
        };
        apply_dir(ord, asc)
    });
}

/// Sort paper orders by column: 0 ID, 1 Side, 2 Market, 3 Outcome, 4 Price,
/// 5 Size, 6 Created.
fn sort_paper_orders(list: &mut [OpenOrder], col: usize, asc: bool) {
    list.sort_by(|a, b| {
        let ord = match col {
            0 => a.id.cmp(&b.id),
            1 => side_rank(a.side).cmp(&side_rank(b.side)),
            2 => a.question.to_lowercase().cmp(&b.question.to_lowercase()),
            3 => a.outcome.to_lowercase().cmp(&b.outcome.to_lowercase()),
            4 => a.price.cmp(&b.price),
            5 => a.size.cmp(&b.size),
            6 => a.created_at.cmp(&b.created_at),
            _ => std::cmp::Ordering::Equal,
        };
        apply_dir(ord, asc)
    });
}

/// Sort live CLOB orders by column: 0 ID, 1 Side, 2 Outcome, 3 Price, 4 Size,
/// 5 Matched, 6 Created. Numeric columns arrive as strings, so parse them.
fn sort_live_orders(list: &mut [LiveOpenOrder], col: usize, asc: bool) {
    let num = |s: &str| s.parse::<f64>().unwrap_or(0.0);
    list.sort_by(|a, b| {
        let ord = match col {
            0 => a.id.cmp(&b.id),
            1 => a.side.to_lowercase().cmp(&b.side.to_lowercase()),
            2 => a.outcome.to_lowercase().cmp(&b.outcome.to_lowercase()),
            3 => num(&a.price)
                .partial_cmp(&num(&b.price))
                .unwrap_or(std::cmp::Ordering::Equal),
            4 => num(&a.size)
                .partial_cmp(&num(&b.size))
                .unwrap_or(std::cmp::Ordering::Equal),
            5 => num(&a.matched)
                .partial_cmp(&num(&b.matched))
                .unwrap_or(std::cmp::Ordering::Equal),
            6 => a.created_at.cmp(&b.created_at),
            _ => std::cmp::Ordering::Equal,
        };
        apply_dir(ord, asc)
    });
}

pub(crate) const SETTING_ROWS: [SettingRow; 11] = [
    SettingRow::Mode,
    SettingRow::AutoSettle,
    SettingRow::GuardAutostart,
    SettingRow::Field(SettingField::Threshold),
    SettingRow::Field(SettingField::Quickbuy),
    SettingRow::Field(SettingField::Quicksell),
    SettingRow::Field(SettingField::Slippage),
    SettingRow::Field(SettingField::TakeProfit),
    SettingRow::Field(SettingField::StopLoss),
    SettingRow::Field(SettingField::Trailing),
    SettingRow::Field(SettingField::CopyPoll),
];

/// Turn a pasted polymarket.com URL into a searchable slug; other queries
/// pass through untouched. E.g.
/// `https://polymarket.com/event/will-x-happen?tid=1` → `will x happen`.
fn normalize_search_query(raw: &str) -> String {
    if !raw.contains("polymarket.com/") {
        return raw.to_string();
    }
    let path = raw.split("polymarket.com/").nth(1).unwrap_or(raw);
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let slug = path
        .split('/')
        .rfind(|seg| !seg.is_empty() && *seg != "event" && *seg != "market")
        .unwrap_or(path);
    slug.replace('-', " ")
}

/// Render a decimal list as a comma string for the editor, e.g. `10, 25, 50`.
fn join_decimals(values: &[Decimal]) -> String {
    values
        .iter()
        .map(|v| v.normalize().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Paper-account reset form: choose a starting balance, wipe everything else.
/// Update confirmation modal: shows the release changelog with a Yes/No choice.
pub(crate) struct UpdateModal {
    pub tag: String,
    pub changelog: String,
    /// Vertical scroll offset into the changelog.
    pub scroll: u16,
    /// Yes/No selection; `true` = Yes (install). Toggled with left/right.
    pub confirm: bool,
}

pub(crate) struct ResetModal {
    pub balance: String,
    /// Turn off all copy-trade followers on reset (avoids instantly re-copying a
    /// high-frequency trader back to square one). Only shown when the roster is
    /// non-empty.
    pub disable_copy: bool,
    /// Whether any followers exist — gates the copy toggle in the modal.
    pub has_copy: bool,
    pub error: Option<String>,
}

/// A field in the follow-wallet (copy-trading) form.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CopyField {
    Wallet,
    Nickname,
    /// Toggle between fixed-dollar and leader-proportional sizing.
    Mode,
    /// Fixed pUSD per copied buy (shown in fixed mode).
    Size,
    /// Multiplier on the leader's own size (shown in ratio mode).
    Ratio,
    MaxDollar,
    MinPrice,
    MaxPrice,
    Slippage,
    MirrorSells,
}

impl CopyField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Wallet => "Wallet (0x…)",
            Self::Nickname => "Nickname",
            Self::Mode => "Sizing (fixed/ratio)",
            Self::Size => "Copy size ($)",
            Self::Ratio => "Ratio (x leader size)",
            Self::MaxDollar => "Max per trade ($)",
            Self::MinPrice => "Min price (0..1)",
            Self::MaxPrice => "Max price (0..1)",
            Self::Slippage => "Slippage (%)",
            Self::MirrorSells => "Mirror sells (y/n)",
        }
    }
}

/// Follow-wallet form: a new copy-trading target with all its rules.
pub(crate) struct CopyModal {
    pub wallet: String,
    pub nickname: String,
    /// When true the form sizes copies as `ratio * leader size`; otherwise it
    /// uses the fixed `size`. Drives which of Size/Ratio the form shows.
    pub use_ratio: bool,
    pub size: String,
    pub ratio: String,
    pub max_dollar: String,
    pub min_price: String,
    pub max_price: String,
    pub slippage: String,
    pub mirror_sells: bool,
    /// Set when reconfiguring an existing follower; `None` means a new follow.
    /// Submitting replaces that follower's rules instead of adding one.
    pub edit_id: Option<String>,
    /// Index into [`CopyModal::fields`].
    pub focus: usize,
    pub error: Option<String>,
}

impl Default for CopyModal {
    fn default() -> Self {
        Self {
            wallet: String::new(),
            nickname: String::new(),
            use_ratio: false,
            size: "25".into(),
            ratio: "1".into(),
            max_dollar: "100".into(),
            min_price: "0".into(),
            max_price: "1".into(),
            slippage: "2".into(),
            mirror_sells: true,
            edit_id: None,
            focus: 0,
            error: None,
        }
    }
}

impl CopyModal {
    /// Pre-fill the form from an existing follower, in edit mode.
    fn from_trader(cfg: &CopyTrader) -> Self {
        let dec = |d: Decimal| d.normalize().to_string();
        Self {
            wallet: cfg.wallet.clone(),
            nickname: cfg.nickname.clone(),
            use_ratio: cfg.copy_ratio.is_some(),
            size: dec(cfg.copy_size_usd),
            ratio: dec(cfg.copy_ratio.unwrap_or(Decimal::ONE)),
            max_dollar: dec(cfg.max_dollar_cap),
            min_price: dec(cfg.price_min),
            max_price: dec(cfg.price_max),
            slippage: dec(cfg.slippage_pct),
            mirror_sells: cfg.mirror_sells,
            edit_id: Some(cfg.id.clone()),
            focus: 0,
            error: None,
        }
    }

    /// Fields shown for the current sizing mode. Size and Ratio are mutually
    /// exclusive, so the list length stays constant and `focus` stays valid.
    pub fn fields(&self) -> [CopyField; 9] {
        let sizing = if self.use_ratio {
            CopyField::Ratio
        } else {
            CopyField::Size
        };
        [
            CopyField::Wallet,
            CopyField::Nickname,
            CopyField::Mode,
            sizing,
            CopyField::MaxDollar,
            CopyField::MinPrice,
            CopyField::MaxPrice,
            CopyField::Slippage,
            CopyField::MirrorSells,
        ]
    }

    /// The text buffer for a field, or `None` for the non-text toggles.
    fn buf(&mut self, f: CopyField) -> Option<&mut String> {
        Some(match f {
            CopyField::Wallet => &mut self.wallet,
            CopyField::Nickname => &mut self.nickname,
            CopyField::Size => &mut self.size,
            CopyField::Ratio => &mut self.ratio,
            CopyField::MaxDollar => &mut self.max_dollar,
            CopyField::MinPrice => &mut self.min_price,
            CopyField::MaxPrice => &mut self.max_price,
            CopyField::Slippage => &mut self.slippage,
            CopyField::Mode | CopyField::MirrorSells => return None,
        })
    }
}

/// Onboarding when live mode starts without a wallet: paste your exported
/// Polymarket private key (logging into your own account). There is no
/// wallet-creation path — the CLI only ever uses a key you already own.
pub(crate) struct OnboardingState {
    /// Text input for importing an existing private key.
    pub import_key: String,
    pub error: Option<String>,
}

/// Modal for wallet management actions in the Settings tab.
pub(crate) struct WalletActionModal {
    pub action: WalletAction,
    /// Private key text buffer (import only).
    pub import_key: String,
    pub error: Option<String>,
    pub confirmed: bool,
}

pub(crate) enum WalletAction {
    Import,
    /// Set the proxy/funder address override (fixes "maker address not allowed").
    SetProxy,
}

/// Forced delay (seconds) before the final logout confirmation unlocks.
pub(crate) const LOGOUT_DELAY_SECS: u64 = 5;

/// Two-step logout guard: first Enter arms the timer, then the final Enter only
/// works after [`LOGOUT_DELAY_SECS`] — so a key removal can't happen by mistake.
pub(crate) struct LogoutModal {
    /// `None` = first confirmation pending; `Some(t)` = armed at time `t`.
    pub armed_at: Option<Instant>,
}

impl LogoutModal {
    /// Seconds left on the unlock timer (0 once the final confirm is allowed).
    pub fn remaining_secs(&self) -> u64 {
        match self.armed_at {
            Some(t) => LOGOUT_DELAY_SECS.saturating_sub(t.elapsed().as_secs()),
            None => LOGOUT_DELAY_SECS,
        }
    }
}

pub(crate) struct App {
    pub view: View,
    pub should_quit: bool,
    pub data: Shared,
    pub account: Arc<Mutex<PaperAccount>>,
    pub copy_engine: CopyEngine,

    pub markets_sel: usize,
    pub positions_sel: usize,
    pub orders_sel: usize,
    pub copytrade_sel: usize,
    pub settings_sel: usize,
    pub history_scroll: usize,
    pub logs_scroll: usize,

    /// Trading settings (mode, presets, slippage, TP/SL).
    pub settings: Settings,
    /// Configured wallet details (live mode); `None` in paper mode.
    pub wallet: Option<WalletInfo>,
    /// Whether the private key is currently revealed on the Settings tab.
    pub reveal_key: bool,
    /// Inline settings editor.
    pub settings_modal: Option<SettingsEditModal>,

    /// Markets search filter (active while `searching`).
    pub search: String,
    pub searching: bool,

    /// Local table filter for the read-only Holdings/History tables (substring
    /// match, no API call). Active while `table_filtering` captures input.
    pub table_filter: String,
    pub table_filtering: bool,
    /// Sort column index + direction for the Holdings/History tables. The index
    /// is clamped to each table's column count at render time.
    pub sort_col: usize,
    pub sort_asc: bool,

    /// The market opened in MarketDetail and which outcome token is focused.
    pub detail: Option<MarketRow>,
    pub detail_token: usize,
    /// Vertical scroll offset for the market-detail left panel (rules text).
    pub detail_scroll: u16,
    /// Index into [`super::data::DETAIL_TIMEFRAMES`] for the price-history chart.
    pub detail_timeframe: usize,

    pub modal: Option<OrderModal>,
    /// Follow-wallet form (Copytrade tab → `n`).
    pub copy_modal: Option<CopyModal>,
    /// Paper-account reset form (Settings tab → Shift+L).
    pub reset_modal: Option<ResetModal>,
    pub update_modal: Option<UpdateModal>,
    /// Onboarding flow when no wallet is configured in live mode.
    pub onboarding: Option<OnboardingState>,
    /// Create/import wallet modal from the Settings tab.
    pub wallet_action_modal: Option<WalletActionModal>,
    /// Two-step logout confirmation (Settings tab → `L`).
    pub logout_modal: Option<LogoutModal>,
    pub status: String,
    /// True in LIVE mode (real wallet + CLOB), false for the paper account.
    pub live: bool,
    /// Condition IDs already submitted for on-chain redemption this session,
    /// so auto-settle never double-sends the transaction.
    pub attempted_redeems: HashSet<String>,
    /// Latest release tag when a newer version exists; `None` if up to date.
    pub update_available: Option<String>,
    /// Set by the `U` key; causes `tui::run` to execute the upgrade after exit.
    pub run_upgrade: bool,
    /// Monotonic frame counter driving UI animations (spinners, matrix rain).
    pub frame: u64,
}

impl App {
    pub fn new(
        data: Shared,
        account: Arc<Mutex<PaperAccount>>,
        copy_engine: CopyEngine,
        live: bool,
    ) -> Self {
        crate::updater::refresh_cache_if_stale();
        let update_available = crate::updater::check_update();
        let status = if let Some(ref tag) = update_available {
            format!("Update {tag} available — press U to install.")
        } else if live {
            "LIVE mode — real funds. Press ? for help, b/s on a market to trade.".to_string()
        } else {
            "PAPER mode — simulated. Press ? for help.".to_string()
        };
        let wallet = if live {
            super::live::wallet_info()
        } else {
            None
        };
        let needs_onboarding = live && wallet.is_none();
        let onboarding = if needs_onboarding {
            Some(OnboardingState {
                import_key: String::new(),
                error: None,
            })
        } else {
            None
        };
        Self {
            view: if needs_onboarding {
                View::Onboarding
            } else {
                View::Dashboard
            },
            should_quit: false,
            data,
            account,
            copy_engine,
            markets_sel: 0,
            positions_sel: 0,
            orders_sel: 0,
            copytrade_sel: 0,
            settings_sel: 0,
            history_scroll: 0,
            logs_scroll: 0,
            settings: settings::load(),
            wallet,
            reveal_key: false,
            settings_modal: None,
            search: String::new(),
            searching: false,
            table_filter: String::new(),
            table_filtering: false,
            sort_col: 0,
            sort_asc: false,
            detail: None,
            detail_scroll: 0,
            detail_token: 0,
            detail_timeframe: 0,
            modal: None,
            copy_modal: None,
            reset_modal: None,
            update_modal: None,
            onboarding,
            logout_modal: None,
            wallet_action_modal: None,
            status,
            live,
            attempted_redeems: HashSet::new(),
            update_available,
            run_upgrade: false,
            frame: 0,
        }
    }

    /// Per-frame housekeeping: refresh the watch set and surface any async
    /// notices (e.g. live-order results) in the status line.
    pub fn pre_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        self.sync_watch();
        let notice = self.data.lock().unwrap().notices.pop();
        if let Some(n) = notice {
            self.status = n;
        }
        // The startup update check reads a possibly-stale on-disk cache; the
        // refresh runs in a background thread. Re-check every ~5s until we see a
        // newer version so the banner and the `U` key light up mid-session.
        if self.update_available.is_none() && self.frame.is_multiple_of(55) {
            self.update_available = crate::updater::check_update();
            if let Some(ref tag) = self.update_available {
                self.status = format!("Update {tag} available — press U to install.");
            }
        }
        self.tick_settlement();
    }

    /// Tokens the data refresher should keep books fresh for.
    pub fn watched_tokens(&self) -> Vec<String> {
        let mut tokens: Vec<String> = self
            .account
            .lock()
            .unwrap()
            .positions
            .keys()
            .cloned()
            .collect();
        if let Some(d) = &self.detail {
            tokens.extend(d.token_ids.iter().cloned());
        }
        tokens.sort();
        tokens.dedup();
        tokens
    }

    /// Push the current watch set to the shared data store each frame.
    pub fn sync_watch(&self) {
        let tokens = self.watched_tokens();
        self.data.lock().unwrap().watch = tokens;
    }

    /// Markets to show: the default top-by-volume list, or live search results
    /// from the Gamma search API when a query is active.
    pub fn filtered_markets(&self) -> Vec<MarketRow> {
        let query = self.search.trim();
        let d = self.data.lock().unwrap();
        if query.is_empty() {
            d.markets.clone()
        } else if d.search_results_query.eq_ignore_ascii_case(query) {
            d.search_results.clone()
        } else {
            // Search in flight — results for this query haven't arrived yet.
            Vec::new()
        }
    }

    /// Fire a Gamma search for the current query (the real search endpoint,
    /// not a filter over the loaded list). Pasted polymarket.com links are
    /// reduced to their slug so a copied URL jumps straight to the market.
    fn run_market_search(&mut self) {
        let raw = self.search.trim().to_string();
        self.markets_sel = 0;
        if raw.is_empty() {
            return;
        }
        let query = normalize_search_query(&raw);
        if query != raw {
            // Show the extracted slug so results visibly match the query.
            self.search = query.clone();
        }
        self.status = format!("Searching markets for “{query}”…");
        super::data::run_search(Arc::clone(&self.data), query);
    }

    /// Whether a search is active but its results haven't arrived yet.
    pub fn search_pending(&self) -> bool {
        let query = self.search.trim();
        if query.is_empty() {
            return false;
        }
        !self
            .data
            .lock()
            .unwrap()
            .search_results_query
            .eq_ignore_ascii_case(query)
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        // Always-available exit: Ctrl+C / Ctrl+Q work everywhere, including
        // inside modals and the search box (raw mode swallows the default
        // Ctrl+C, so we handle it ourselves).
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('q'))
        {
            self.should_quit = true;
            return;
        }
        // Onboarding captures all input first.
        if self.onboarding.is_some() {
            self.onboarding_key(key);
            return;
        }
        // Wallet action modal captures input.
        if self.wallet_action_modal.is_some() {
            self.wallet_action_modal_key(key);
            return;
        }
        // Logout confirmation captures input.
        if self.logout_modal.is_some() {
            self.logout_modal_key(key);
            return;
        }
        // Order/copy/reset/settings modals capture all input.
        if self.modal.is_some() {
            self.modal_key(key);
            return;
        }
        if self.copy_modal.is_some() {
            self.copy_modal_key(key);
            return;
        }
        if self.reset_modal.is_some() {
            self.reset_modal_key(key);
            return;
        }
        if self.update_modal.is_some() {
            self.update_modal_key(key);
            return;
        }
        if self.settings_modal.is_some() {
            self.settings_modal_key(key);
            return;
        }
        // Search box on Markets captures input.
        if self.searching {
            match key.code {
                KeyCode::Esc => {
                    self.searching = false;
                    self.search.clear();
                }
                KeyCode::Enter => {
                    self.searching = false;
                    self.run_market_search();
                }
                KeyCode::Backspace => {
                    self.search.pop();
                }
                KeyCode::Char(c) => self.search.push(c),
                _ => {}
            }
            return;
        }
        // Local table filter (Holdings/History) captures input live, no API call.
        if self.table_filtering {
            match key.code {
                KeyCode::Esc => {
                    self.table_filtering = false;
                    self.table_filter.clear();
                }
                KeyCode::Enter => self.table_filtering = false,
                KeyCode::Backspace => {
                    self.table_filter.pop();
                }
                KeyCode::Char(c) => self.table_filter.push(c),
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => {
                self.status = "Tab/1-9 switch views · ↑↓/jk move · Enter open · b/s order · c cancel · q or Ctrl+C quit".to_string();
            }
            KeyCode::Tab => self.cycle_tab(1),
            KeyCode::BackTab => self.cycle_tab(-1),
            KeyCode::Char(c @ '1'..='9') => {
                let idx = c as usize - '1' as usize;
                if idx < View::TABS.len() {
                    self.view = View::TABS[idx];
                    self.reset_table_view();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_sel(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_sel(-1),
            KeyCode::Enter => self.activate(),
            KeyCode::Esc => {
                if self.view == View::MarketDetail {
                    self.view = View::Markets;
                } else if self.view == View::Markets && !self.search.is_empty() {
                    self.search.clear();
                    self.markets_sel = 0;
                    self.status = "Search cleared.".to_string();
                } else if !self.table_filter.is_empty() {
                    self.table_filter.clear();
                    self.status = "Filter cleared.".to_string();
                }
            }
            KeyCode::Char('/') if self.view == View::Markets => {
                self.searching = true;
                self.search.clear();
            }
            // Local substring filter on the tabular views.
            KeyCode::Char('/')
                if matches!(
                    self.view,
                    View::Portfolio | View::History | View::Positions | View::Orders
                ) =>
            {
                self.table_filtering = true;
                self.table_filter.clear();
                // Filtering narrows from the top; keep the cursor in range.
                self.positions_sel = 0;
                self.orders_sel = 0;
            }
            // Column sort: `o` cycles the sort column, `O` reverses direction.
            // History is filter-only (kept newest-first), so it's excluded.
            KeyCode::Char('o')
                if matches!(self.view, View::Portfolio | View::Positions | View::Orders) =>
            {
                let cols = match self.view {
                    View::Positions => POSITIONS_SORT_COLS,
                    View::Orders => ORDERS_SORT_COLS,
                    _ => HOLDINGS_SORT_COLS,
                };
                self.sort_col = (self.sort_col + 1) % cols;
            }
            KeyCode::Char('O')
                if matches!(self.view, View::Portfolio | View::Positions | View::Orders) =>
            {
                self.sort_asc = !self.sort_asc;
            }
            KeyCode::Left | KeyCode::Char('h') if self.view == View::MarketDetail => {
                if self.detail_token > 0 {
                    self.detail_token -= 1;
                    self.request_price_history();
                }
            }
            KeyCode::Right | KeyCode::Char('l') if self.view == View::MarketDetail => {
                if let Some(d) = &self.detail
                    && self.detail_token + 1 < d.token_ids.len()
                {
                    self.detail_token += 1;
                    self.request_price_history();
                }
            }
            // Cycle the price-history chart timeframe (5m / 30m / 1h / 1d).
            KeyCode::Char('t') if self.view == View::MarketDetail => {
                self.detail_timeframe =
                    (self.detail_timeframe + 1) % super::data::DETAIL_TIMEFRAMES.len();
                self.request_price_history();
            }
            KeyCode::Char('b') if self.view == View::MarketDetail => {
                self.open_modal(TradeSide::Buy)
            }
            KeyCode::Char('s') if self.view == View::MarketDetail => {
                self.open_modal(TradeSide::Sell)
            }
            // Trade straight from the Positions tab (paper and live).
            KeyCode::Char('b') if self.view == View::Positions => {
                self.open_modal_for_position(TradeSide::Buy)
            }
            KeyCode::Char('s') if self.view == View::Positions => {
                self.open_modal_for_position(TradeSide::Sell)
            }
            // Claim a resolved position's payout (manual-claim mode).
            KeyCode::Char('r') if self.view == View::Positions => self.redeem_selected_position(),
            KeyCode::Char('c') if self.view == View::Orders => self.cancel_selected_order(),
            // Copy-trading controls.
            KeyCode::Char('n') if self.view == View::Copytrade => {
                self.copy_modal = Some(CopyModal::default());
            }
            // Reconfigure the selected follower (e.g. switch fixed -> ratio).
            KeyCode::Char('c') if self.view == View::Copytrade => {
                self.open_copy_edit();
            }
            KeyCode::Char('s') if self.view == View::Copytrade => {
                self.copytrade_action(CopyAct::Start)
            }
            KeyCode::Char('x') if self.view == View::Copytrade => {
                self.copytrade_action(CopyAct::Stop)
            }
            KeyCode::Char('e') if self.view == View::Copytrade => {
                self.copytrade_action(CopyAct::Enable)
            }
            KeyCode::Char('d') if self.view == View::Copytrade => {
                self.copytrade_action(CopyAct::Disable)
            }
            KeyCode::Char('D') | KeyCode::Delete if self.view == View::Copytrade => {
                self.copytrade_action(CopyAct::Delete)
            }
            // Settings: reveal/hide the private key (live wallet).
            KeyCode::Char('w') if self.view == View::Settings => {
                if self.wallet.is_some() {
                    self.reveal_key = !self.reveal_key;
                    self.status = if self.reveal_key {
                        "⚠ Private key revealed — anyone seeing your screen can drain the wallet. Press w to hide.".into()
                    } else {
                        "Private key hidden.".into()
                    };
                } else {
                    self.status = "No wallet configured (paper mode).".into();
                }
            }
            // Settings: open wallet profile in browser.
            KeyCode::Char('o') if self.view == View::Settings && self.live => {
                if let Some(w) = &self.wallet {
                    let url = format!("https://polymarket.com/profile/{}", w.eoa);
                    match webbrowser::open(&url) {
                        Ok(()) => self.status = format!("Opened {url} in browser."),
                        Err(e) => self.status = format!("Failed to open browser: {e}"),
                    }
                }
            }
            // Settings: approve contracts, check approvals, deposit info.
            KeyCode::Char('a') if self.view == View::Settings && self.live => {
                self.status = "Approving all contracts (sending up to 12 on-chain txns)…".into();
                let shared = Arc::clone(&self.data);
                tokio::spawn(async move {
                    let msg = match crate::commands::approve::tui_set_approvals().await {
                        Ok(s) => s,
                        Err(e) => format!("Approve failed: {e}"),
                    };
                    shared.lock().unwrap().notices.push(msg);
                });
            }
            KeyCode::Char('c') if self.view == View::Settings && self.live => {
                let shared = Arc::clone(&self.data);
                tokio::spawn(async move {
                    let msg = match crate::commands::approve::tui_check_approvals().await {
                        Ok(s) => s,
                        Err(e) => format!("Check approvals failed: {e}"),
                    };
                    shared.lock().unwrap().notices.push(msg);
                });
                self.status = "Checking approvals…".into();
            }
            KeyCode::Char('d') if self.view == View::Settings && self.live => {
                let shared = Arc::clone(&self.data);
                tokio::spawn(async move {
                    let msg = match crate::commands::bridge::tui_deposit_address().await {
                        Ok(s) => s,
                        Err(e) => format!("Deposit address lookup failed: {e}"),
                    };
                    shared.lock().unwrap().notices.push(msg);
                });
                self.status = "Fetching deposit address…".into();
            }
            // Settings: import (log into) a wallet (live mode).
            KeyCode::Char('m') if self.view == View::Settings && self.live => {
                self.wallet_action_modal = Some(WalletActionModal {
                    action: WalletAction::Import,
                    import_key: String::new(),
                    error: None,
                    confirmed: false,
                });
            }
            // Settings: set the proxy/funder address override (fixes the CLOB
            // "maker address not allowed" error for web-created accounts).
            KeyCode::Char('x') if self.view == View::Settings && self.live => {
                if self.wallet.is_some() {
                    let prefill = config::resolve_proxy_address()
                        .ok()
                        .flatten()
                        .unwrap_or_default();
                    self.wallet_action_modal = Some(WalletActionModal {
                        action: WalletAction::SetProxy,
                        import_key: prefill,
                        error: None,
                        confirmed: false,
                    });
                } else {
                    self.status = "Import a wallet first (m).".into();
                }
            }
            // Settings: Shift+L => live logout, paper reset shortcut.
            KeyCode::Char('L') if self.view == View::Settings => {
                if self.live {
                    if self.wallet.is_some() {
                        self.logout_modal = Some(LogoutModal { armed_at: None });
                    } else {
                        self.status = "No wallet to log out of.".into();
                    }
                } else {
                    self.open_reset_modal();
                    self.status =
                        "Paper reset armed. Enter a new starting balance and press Enter.".into();
                }
            }
            // Settings: cycle the signature type (eoa → proxy → gnosis-safe).
            KeyCode::Char('y') if self.view == View::Settings && self.live => {
                if self.wallet.is_some() {
                    self.cycle_signature_type();
                } else {
                    self.status = "Import a wallet first (m).".into();
                }
            }
            KeyCode::Char('U') if self.update_available.is_some() => {
                let tag = self.update_available.clone().unwrap_or_default();
                let changelog = crate::updater::changelog()
                    .unwrap_or_else(|| "Release notes unavailable.".into());
                self.update_modal = Some(UpdateModal {
                    tag,
                    changelog,
                    scroll: 0,
                    confirm: true,
                });
            }
            _ => {}
        }
    }

    fn cycle_tab(&mut self, dir: i32) {
        let cur = View::TABS.iter().position(|v| *v == self.view).unwrap_or(0);
        let n = View::TABS.len() as i32;
        let next = (cur as i32 + dir).rem_euclid(n) as usize;
        self.view = View::TABS[next];
        self.reset_table_view();
    }

    /// Clear the shared table filter + sort when leaving a tab, so a filter set
    /// on one table doesn't silently hide rows on the next.
    pub(crate) fn reset_table_view(&mut self) {
        self.table_filtering = false;
        self.table_filter.clear();
        self.sort_col = 0;
        self.sort_asc = false;
        self.positions_sel = 0;
        self.orders_sel = 0;
    }

    fn move_sel(&mut self, dir: i32) {
        let step = |sel: &mut usize, len: usize| {
            if len == 0 {
                *sel = 0;
                return;
            }
            let n = (*sel as i32 + dir).clamp(0, len as i32 - 1);
            *sel = n as usize;
        };
        match self.view {
            View::Markets => {
                let len = self.filtered_markets().len();
                step(&mut self.markets_sel, len);
            }
            View::Positions => {
                let (open, resolved) = self.ordered_positions();
                step(&mut self.positions_sel, open.len() + resolved.len());
            }
            View::Orders => {
                let len = if self.live {
                    self.ordered_live_orders().len()
                } else {
                    self.ordered_paper_orders().len()
                };
                step(&mut self.orders_sel, len);
            }
            View::Copytrade => {
                let len = self.copy_engine.snapshot().len();
                step(&mut self.copytrade_sel, len);
            }
            View::Settings => {
                step(&mut self.settings_sel, SETTING_ROWS.len());
            }
            View::History => {
                if dir > 0 {
                    self.history_scroll += 1;
                } else {
                    self.history_scroll = self.history_scroll.saturating_sub(1);
                }
            }
            View::Logs => {
                if dir > 0 {
                    self.logs_scroll += 1;
                } else {
                    self.logs_scroll = self.logs_scroll.saturating_sub(1);
                }
            }
            View::MarketDetail => {
                if dir > 0 {
                    self.detail_scroll += 1;
                } else {
                    self.detail_scroll = self.detail_scroll.saturating_sub(1);
                }
            }
            _ => {}
        }
    }

    /// Ask the background loop to fetch price history for the focused outcome
    /// at the current timeframe.
    fn request_price_history(&self) {
        let Some(d) = &self.detail else { return };
        let Some(token) = d.token_ids.get(self.detail_token) else {
            return;
        };
        super::data::run_price_history(
            Arc::clone(&self.data),
            token.clone(),
            self.detail_timeframe,
        );
    }

    fn activate(&mut self) {
        match self.view {
            View::Markets => {
                let markets = self.filtered_markets();
                if let Some(row) = markets.get(self.markets_sel) {
                    self.detail = Some(row.clone());
                    self.detail_token = 0;
                    self.detail_scroll = 0;
                    self.view = View::MarketDetail;
                    self.status = format!("Opened: {}", row.question);
                    self.request_price_history();
                }
            }
            View::Settings => self.activate_setting(),
            _ => {}
        }
    }

    // --- Settings editing --------------------------------------------------

    /// Act on the selected Settings row: cycle the trading mode in place, or
    /// open the inline editor for a value.
    fn activate_setting(&mut self) {
        let Some(row) = SETTING_ROWS.get(self.settings_sel) else {
            return;
        };
        match row {
            SettingRow::Mode => {
                self.settings.trading_mode = self.settings.trading_mode.next();
                self.persist_settings();
                self.status = format!(
                    "Trading mode → {} ({}).",
                    self.settings.trading_mode,
                    self.settings.trading_mode.describe()
                );
            }
            SettingRow::AutoSettle => {
                self.settings.auto_settle = !self.settings.auto_settle;
                self.persist_settings();
                self.status = if self.settings.auto_settle {
                    "Resolved markets now settle to cash automatically.".into()
                } else {
                    "Resolved markets now wait for a manual claim (r on Positions).".into()
                };
            }
            SettingRow::GuardAutostart => {
                let result = if crate::commands::guard::autostart_enabled() {
                    crate::commands::guard::autostart_off()
                } else {
                    crate::commands::guard::autostart_on()
                };
                self.status = match result {
                    Ok(()) if crate::commands::guard::autostart_enabled() => {
                        "Guard worker will start at login.".into()
                    }
                    Ok(()) => "Guard worker autostart disabled.".into(),
                    Err(e) => format!("Autostart toggle failed: {e}"),
                };
            }
            SettingRow::Field(field) => {
                let input = self.setting_current_value(*field);
                self.settings_modal = Some(SettingsEditModal {
                    field: *field,
                    input,
                    error: None,
                });
            }
        }
    }

    /// The current value of an editable setting, pre-filled into the editor.
    pub(crate) fn setting_current_value(&self, field: SettingField) -> String {
        let s = &self.settings;
        let opt = |v: Option<Decimal>| v.map(|d| d.normalize().to_string()).unwrap_or_default();
        match field {
            SettingField::Threshold => s.confirm_threshold_usd.normalize().to_string(),
            SettingField::Quickbuy => join_decimals(&s.quickbuy_presets),
            SettingField::Quicksell => join_decimals(&s.quicksell_presets),
            SettingField::Slippage => s.slippage_pct.normalize().to_string(),
            SettingField::TakeProfit => opt(s.default_take_profit_pct),
            SettingField::StopLoss => opt(s.default_stop_loss_pct),
            SettingField::Trailing => opt(s.default_trailing_stop_pct),
            SettingField::CopyPoll => s.copy_poll_secs.to_string(),
        }
    }

    fn settings_modal_key(&mut self, key: KeyEvent) {
        let Some(m) = self.settings_modal.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.settings_modal = None,
            KeyCode::Backspace => {
                m.input.pop();
            }
            KeyCode::Char(c) if c.is_ascii_digit() || c == '.' || c == ',' || c == ' ' => {
                m.input.push(c);
            }
            KeyCode::Enter => self.submit_setting(),
            _ => {}
        }
    }

    fn submit_setting(&mut self) {
        let (field, raw) = match self.settings_modal.as_ref() {
            Some(m) => (m.field, m.input.trim().to_string()),
            None => return,
        };
        // An optional percent: blank clears it.
        let parse_opt_pct = |raw: &str| -> Result<Option<Decimal>, String> {
            if raw.is_empty() {
                return Ok(None);
            }
            match Decimal::from_str(raw) {
                Ok(v) if v > Decimal::ZERO => Ok(Some(v)),
                Ok(_) => Err("Enter a positive percent, or blank to turn off.".into()),
                Err(_) => Err(format!("'{raw}' is not a number.")),
            }
        };
        let result: Result<(), String> = (|| {
            match field {
                SettingField::Threshold => {
                    let v = Decimal::from_str(&raw)
                        .map_err(|_| "Enter a dollar amount.".to_string())?;
                    if v < Decimal::ZERO {
                        return Err("Threshold cannot be negative.".into());
                    }
                    self.settings.confirm_threshold_usd = v;
                }
                SettingField::Quickbuy => {
                    self.settings.quickbuy_presets =
                        settings::parse_number_list(&raw).map_err(|e| e.to_string())?;
                }
                SettingField::Quicksell => {
                    self.settings.quicksell_presets =
                        settings::parse_number_list(&raw).map_err(|e| e.to_string())?;
                }
                SettingField::Slippage => {
                    let v = Decimal::from_str(&raw).map_err(|_| "Enter a percent.".to_string())?;
                    if v < Decimal::ZERO {
                        return Err("Slippage cannot be negative.".into());
                    }
                    self.settings.slippage_pct = v;
                }
                SettingField::TakeProfit => {
                    self.settings.default_take_profit_pct = parse_opt_pct(&raw)?;
                }
                SettingField::StopLoss => {
                    self.settings.default_stop_loss_pct = parse_opt_pct(&raw)?;
                }
                SettingField::Trailing => {
                    self.settings.default_trailing_stop_pct = parse_opt_pct(&raw)?;
                }
                SettingField::CopyPoll => {
                    let v: u64 = raw
                        .parse()
                        .map_err(|_| "Enter whole seconds (e.g. 5).".to_string())?;
                    self.settings.copy_poll_secs = v.max(1);
                    self.copy_engine.set_interval(v.max(1));
                }
            }
            Ok(())
        })();

        match result {
            Ok(()) => {
                self.persist_settings();
                self.settings_modal = None;
                self.status = "Setting saved.".into();
            }
            Err(e) => {
                if let Some(m) = self.settings_modal.as_mut() {
                    m.error = Some(e);
                }
            }
        }
    }

    fn persist_settings(&self) {
        let _ = settings::save(&self.settings);
    }

    fn open_reset_modal(&mut self) {
        let current = self.account.lock().unwrap().initial_balance;
        let prefill = if current > Decimal::ZERO {
            current.round_dp(0).to_string()
        } else {
            default_starting_balance().round_dp(0).to_string()
        };
        self.reset_modal = Some(ResetModal {
            balance: prefill,
            disable_copy: true,
            has_copy: !self.copy_engine.snapshot().is_empty(),
            error: None,
        });
    }

    // --- Order modal -------------------------------------------------------

    fn open_modal(&mut self, side: TradeSide) {
        let Some(d) = &self.detail else { return };
        let Some(token_id) = d.token_ids.get(self.detail_token) else {
            return;
        };
        let token_id = token_id.clone();
        let outcome = d
            .outcomes
            .get(self.detail_token)
            .cloned()
            .unwrap_or_else(|| format!("Outcome {}", self.detail_token + 1));
        let question = d.question.clone();
        self.open_modal_with(token_id, question, outcome, side);
    }

    /// Open the order form for the position selected on the Positions tab.
    fn open_modal_for_position(&mut self, side: TradeSide) {
        let Some(p) = self.selected_position() else {
            self.status = "No position selected.".into();
            return;
        };
        if self
            .data
            .lock()
            .unwrap()
            .resolutions
            .contains_key(&p.token_id)
        {
            self.status = "Market resolved — press r to redeem instead of trading.".into();
            return;
        }
        self.open_modal_with(p.token_id, p.question, p.outcome, side);
    }

    /// True when a row with this market/outcome passes the active table filter.
    fn row_matches_filter(&self, question: &str, outcome: &str) -> bool {
        let f = self.table_filter.to_lowercase();
        f.is_empty() || question.to_lowercase().contains(&f) || outcome.to_lowercase().contains(&f)
    }

    /// Positions in display order: open (live) first, then resolved
    /// (redeemable), each passed through the active filter and column sort.
    ///
    /// This is the single source of truth for the Positions tab: the render
    /// path, the cursor length ([`move_sel`]), and the b/s/r selection→action
    /// mapping ([`selected_position`], [`redeem_selected_position`]) all consume
    /// it, so a filtered/sorted row can never resolve to a different position
    /// than the one highlighted.
    pub(crate) fn ordered_positions(&self) -> (Vec<PositionView>, Vec<PositionView>) {
        let (marks, resolutions) = {
            let d = self.data.lock().unwrap();
            let marks: std::collections::BTreeMap<String, Decimal> =
                d.marks.iter().map(|(k, v)| (k.clone(), *v)).collect();
            let resolved: HashSet<String> = d.resolutions.keys().cloned().collect();
            (marks, resolved)
        };
        let view = {
            let acct = self.account.lock().unwrap();
            paper_engine::portfolio_view(&acct, &marks)
        };
        let (mut open, mut resolved): (Vec<PositionView>, Vec<PositionView>) = view
            .positions
            .into_iter()
            .filter(|p| self.row_matches_filter(&p.position.question, &p.position.outcome))
            .partition(|p| !resolutions.contains(&p.position.token_id));
        sort_position_views(&mut open, self.sort_col, self.sort_asc);
        sort_position_views(&mut resolved, self.sort_col, self.sort_asc);
        (open, resolved)
    }

    /// The position under the Positions-tab cursor, resolved through the same
    /// ordering the table renders (see [`ordered_positions`]).
    fn selected_position(&self) -> Option<Position> {
        let (open, resolved) = self.ordered_positions();
        open.into_iter()
            .chain(resolved)
            .nth(self.positions_sel)
            .map(|p| p.position)
    }

    /// Paper open orders in display order (filtered + sorted). Shared by the
    /// Orders render and the cancel-selected-order action.
    pub(crate) fn ordered_paper_orders(&self) -> Vec<OpenOrder> {
        let mut orders: Vec<OpenOrder> = self
            .account
            .lock()
            .unwrap()
            .open_orders
            .iter()
            .filter(|o| self.row_matches_filter(&o.question, &o.outcome))
            .cloned()
            .collect();
        sort_paper_orders(&mut orders, self.sort_col, self.sort_asc);
        orders
    }

    /// Live CLOB orders in display order (filtered + sorted).
    pub(crate) fn ordered_live_orders(&self) -> Vec<LiveOpenOrder> {
        let mut orders: Vec<LiveOpenOrder> = self
            .data
            .lock()
            .unwrap()
            .live_orders
            .iter()
            .filter(|o| self.row_matches_filter("", &o.outcome))
            .cloned()
            .collect();
        sort_live_orders(&mut orders, self.sort_col, self.sort_asc);
        orders
    }

    fn open_modal_with(
        &mut self,
        token_id: String,
        question: String,
        outcome: String,
        side: TradeSide,
    ) {
        // Prefill TP/SL on buys from the configured defaults.
        let pct = |v: Option<Decimal>| v.map(|d| d.normalize().to_string()).unwrap_or_default();
        let (tp, sl) = if side == TradeSide::Buy {
            (
                pct(self.settings.default_take_profit_pct),
                pct(self.settings.default_stop_loss_pct),
            )
        } else {
            (String::new(), String::new())
        };
        let held = self
            .account
            .lock()
            .unwrap()
            .positions
            .get(&token_id)
            .map_or(Decimal::ZERO, |p| p.size);
        self.modal = Some(OrderModal {
            token_id,
            question,
            outcome,
            side,
            kind: OrderKind::Market,
            amount: String::new(),
            price: String::new(),
            size: String::new(),
            tp,
            sl,
            field: ModalField::Amount,
            error: None,
            preset_idx: 0,
            held,
            awaiting_confirm: false,
        });
    }

    fn modal_key(&mut self, key: KeyEvent) {
        // Enter at the confirmation gate sends; Esc anywhere cancels.
        match key.code {
            KeyCode::Esc => {
                self.modal = None;
                return;
            }
            KeyCode::Enter => {
                self.submit_modal();
                return;
            }
            KeyCode::Char('p') => {
                self.apply_preset();
                return;
            }
            _ => {}
        }
        let Some(m) = self.modal.as_mut() else { return };
        // Any edit invalidates a pending confirmation.
        match key.code {
            KeyCode::Char('m') => {
                m.kind = OrderKind::Market;
                m.field = ModalField::Amount;
                m.awaiting_confirm = false;
            }
            KeyCode::Char('L') => {
                m.kind = OrderKind::Limit;
                m.field = ModalField::Price;
                m.awaiting_confirm = false;
            }
            KeyCode::Tab => {
                m.field = next_field(m.kind, m.side, m.field);
            }
            KeyCode::Backspace => {
                field_mut(m).pop();
                m.awaiting_confirm = false;
            }
            KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => {
                field_mut(m).push(c);
                m.awaiting_confirm = false;
            }
            _ => {}
        }
    }

    /// `p` quick-fill: cycle through quickbuy ($) presets on buys, or quicksell
    /// (% of the held position) presets on sells.
    fn apply_preset(&mut self) {
        let Some(m) = self.modal.as_mut() else { return };
        m.awaiting_confirm = false;
        match m.side {
            TradeSide::Buy => {
                let presets = &self.settings.quickbuy_presets;
                if presets.is_empty() {
                    return;
                }
                let v = presets[m.preset_idx % presets.len()];
                m.preset_idx += 1;
                let s = v.normalize().to_string();
                match m.kind {
                    OrderKind::Market => {
                        m.amount = s;
                        m.field = ModalField::Amount;
                    }
                    // For limit buys the preset seeds the size (shares).
                    OrderKind::Limit => {
                        m.size = s;
                        m.field = ModalField::Size;
                    }
                    OrderKind::Settlement => {}
                }
            }
            TradeSide::Sell => {
                let presets = &self.settings.quicksell_presets;
                if presets.is_empty() || m.held <= Decimal::ZERO {
                    return;
                }
                let pct = presets[m.preset_idx % presets.len()];
                m.preset_idx += 1;
                // 100% sells the exact held size; rounding would leave a dust
                // residual that keeps the position alive showing 0.0 shares.
                let shares = if pct >= Decimal::ONE_HUNDRED {
                    m.held
                } else {
                    (m.held * pct / Decimal::ONE_HUNDRED).round_dp(2)
                };
                let s = shares.normalize().to_string();
                match m.kind {
                    OrderKind::Market => {
                        m.amount = s;
                        m.field = ModalField::Amount;
                    }
                    OrderKind::Limit => {
                        m.size = s;
                        m.field = ModalField::Size;
                    }
                    OrderKind::Settlement => {}
                }
            }
        }
    }

    fn submit_modal(&mut self) {
        // Trading-mode confirmation gate (Cautious / Standard threshold).
        if self.confirm_gate() {
            return;
        }
        if self.live {
            self.submit_live_order();
            return;
        }
        let Some(m) = self.modal.as_ref() else { return };
        let token_id = m.token_id.clone();
        let meta = MarketMeta {
            question: m.question.clone(),
            outcome: m.outcome.clone(),
        };
        let book = {
            let d = self.data.lock().unwrap();
            d.book(&token_id).cloned()
        };
        let Some(book) = book else {
            if let Some(m) = self.modal.as_mut() {
                m.error = Some("No live book yet — wait for a refresh.".into());
            }
            return;
        };
        let now = Utc::now();
        let quote = Quote {
            best_bid: book.best_bid,
            best_ask: book.best_ask,
        };
        let slippage = self.settings.slippage_pct;
        let result: anyhow::Result<String> = (|| {
            let mut acct = self.account.lock().unwrap();
            match (m.kind, m.side) {
                (OrderKind::Market, TradeSide::Buy) => {
                    let usd = parse_dec(&m.amount)?;
                    paper_engine::check_slippage(&book.asks, TradeSide::Buy, usd, slippage)?;
                    let t = paper_engine::market_buy(
                        &mut acct, &token_id, &meta, &book.asks, &book.bids, usd, now,
                    )?;
                    Ok(format!(
                        "Bought {} @ {}",
                        t.size.round_dp(2),
                        t.price.round_dp(4)
                    ))
                }
                (OrderKind::Market, TradeSide::Sell) => {
                    let shares = parse_dec(&m.amount)?;
                    paper_engine::check_slippage(&book.bids, TradeSide::Sell, shares, slippage)?;
                    let t =
                        paper_engine::market_sell(&mut acct, &token_id, &book.bids, shares, now)?;
                    Ok(format!(
                        "Sold {} @ {} (pnl {})",
                        t.size.round_dp(2),
                        t.price.round_dp(4),
                        t.realized_pnl.unwrap_or_default().round_dp(2)
                    ))
                }
                (OrderKind::Limit, TradeSide::Buy) => {
                    let price = parse_dec(&m.price)?;
                    let size = parse_dec(&m.size)?;
                    match paper_engine::limit_buy(
                        &mut acct, &token_id, &meta, quote, price, size, now,
                    )? {
                        paper_engine::LimitOutcome::Filled(t) => Ok(format!(
                            "Limit buy filled {} @ {}",
                            t.size.round_dp(2),
                            t.price
                        )),
                        paper_engine::LimitOutcome::Resting(o) => Ok(format!(
                            "Limit buy resting #{} {} @ {}",
                            o.id,
                            o.size.round_dp(2),
                            o.price
                        )),
                    }
                }
                // The modal only ever builds market/limit orders.
                (OrderKind::Settlement, _) => {
                    unreachable!("settlement is not an order form kind")
                }
                (OrderKind::Limit, TradeSide::Sell) => {
                    let price = parse_dec(&m.price)?;
                    let size = parse_dec(&m.size)?;
                    match paper_engine::limit_sell(&mut acct, &token_id, quote, price, size, now)? {
                        paper_engine::LimitOutcome::Filled(t) => Ok(format!(
                            "Limit sell filled {} @ {}",
                            t.size.round_dp(2),
                            t.price
                        )),
                        paper_engine::LimitOutcome::Resting(o) => Ok(format!(
                            "Limit sell resting #{} {} @ {}",
                            o.id,
                            o.size.round_dp(2),
                            o.price
                        )),
                    }
                }
            }
        })();

        match result {
            Ok(msg) => {
                let _ = store::save(&self.account.lock().unwrap());
                let exit = self.attach_exit_from_modal();
                self.status = match exit {
                    Some(note) => format!("[paper] {msg} · {note}"),
                    None => format!("[paper] {msg}"),
                };
                self.modal = None;
            }
            Err(e) => {
                if let Some(m) = self.modal.as_mut() {
                    m.error = Some(e.to_string());
                }
            }
        }
    }

    /// Whether the order needs confirmation and we've just asked for it (so the
    /// caller should stop and wait for the next Enter). Sets the prompt.
    fn confirm_gate(&mut self) -> bool {
        let already = self.modal.as_ref().is_some_and(|m| m.awaiting_confirm);
        if already {
            return false; // confirmed — proceed
        }
        let Some(notional) = self.order_notional() else {
            return false; // can't size it; let downstream validate
        };
        if !self.settings.requires_confirmation(notional) {
            return false;
        }
        let mode = self.settings.trading_mode;
        if let Some(m) = self.modal.as_mut() {
            m.awaiting_confirm = true;
            m.error = None;
            self.status = format!(
                "Confirm {} ${:.2} [{} mode] — press Enter to send, Esc to cancel.",
                m.side, notional, mode
            );
        }
        true
    }

    /// Best-effort notional (pUSD) of the order in the open modal, for the
    /// confirmation gate. `None` when it can't be sized yet.
    fn order_notional(&self) -> Option<Decimal> {
        let m = self.modal.as_ref()?;
        match m.kind {
            OrderKind::Market => {
                let amt = Decimal::from_str(m.amount.trim()).ok()?;
                match m.side {
                    TradeSide::Buy => Some(amt), // pUSD spent
                    TradeSide::Sell => {
                        // shares * best bid (fallback to mid).
                        let d = self.data.lock().unwrap();
                        let mark = d.book(&m.token_id).and_then(|b| b.best_bid.or(b.best_ask));
                        mark.map(|p| (p * amt).abs())
                    }
                }
            }
            OrderKind::Limit => {
                let price = Decimal::from_str(m.price.trim()).ok()?;
                let size = Decimal::from_str(m.size.trim()).ok()?;
                Some(price * size)
            }
            OrderKind::Settlement => None,
        }
    }

    /// After a buy, attach (or replace) a take-profit/stop-loss guard on the
    /// token using the modal's TP/SL fields plus the default trailing stop.
    /// Returns a short note for the status line, or `None` if nothing attached.
    fn attach_exit_from_modal(&mut self) -> Option<String> {
        let m = self.modal.as_ref()?;
        if m.side != TradeSide::Buy {
            return None;
        }
        let token_id = m.token_id.clone();
        let tp = parse_opt_dec(&m.tp);
        let sl = parse_opt_dec(&m.sl);
        let trailing = self.settings.default_trailing_stop_pct;
        if tp.is_none() && sl.is_none() && trailing.is_none() {
            return None;
        }
        // One guard per token; arm replaces any existing one. The TUI data
        // refresher watches the position and sells when a threshold is crossed.
        match crate::guard::arm(&token_id, self.live, tp, sl, trailing) {
            Ok(()) => {
                let g = crate::guard::Guard {
                    token_id,
                    live: self.live,
                    take_profit_pct: tp,
                    stop_loss_pct: sl,
                    trailing_stop_pct: trailing,
                };
                Some(format!("guard armed ({})", g.describe()))
            }
            Err(_) => None,
        }
    }

    /// Build a real order from the modal and submit it to the CLOB in the
    /// background; the result lands in the status line and the Logs tab.
    fn submit_live_order(&mut self) {
        let (token_id, side, kind, amount_s, price_s, size_s) = {
            let Some(m) = self.modal.as_ref() else { return };
            (
                m.token_id.clone(),
                m.side,
                m.kind,
                m.amount.clone(),
                m.price.clone(),
                m.size.clone(),
            )
        };
        let order = match kind {
            OrderKind::Market => match parse_dec(&amount_s) {
                Ok(amount) => {
                    // Estimate slippage from the freshest cached book before
                    // sending the FOK order to the CLOB.
                    let levels = {
                        let d = self.data.lock().unwrap();
                        d.book(&token_id).map(|b| match side {
                            TradeSide::Buy => b.asks.clone(),
                            TradeSide::Sell => b.bids.clone(),
                        })
                    };
                    if let Some(levels) = levels
                        && let Err(e) = paper_engine::check_slippage(
                            &levels,
                            side,
                            amount,
                            self.settings.slippage_pct,
                        )
                    {
                        return self.set_modal_error(e.to_string());
                    }
                    LiveOrder::Market {
                        token_id,
                        side,
                        amount,
                    }
                }
                Err(e) => return self.set_modal_error(e.to_string()),
            },
            OrderKind::Limit => {
                let price = match parse_dec(&price_s) {
                    Ok(p) => p,
                    Err(e) => return self.set_modal_error(e.to_string()),
                };
                let size = match parse_dec(&size_s) {
                    Ok(s) => s,
                    Err(e) => return self.set_modal_error(e.to_string()),
                };
                LiveOrder::Limit {
                    token_id,
                    side,
                    price,
                    size,
                }
            }
            OrderKind::Settlement => {
                unreachable!("settlement is not an order form kind")
            }
        };

        let shared = Arc::clone(&self.data);
        tokio::spawn(async move {
            let msg = match super::live::place(order).await {
                Ok(s) => s,
                Err(e) => friendly_live_order_error(e),
            };
            shared.lock().unwrap().notices.push(msg);
        });
        // Arm a TP/SL guard now; the data refresher watches the live position
        // once it hydrates and sells when a threshold is crossed.
        let exit = self.attach_exit_from_modal();
        self.status = match exit {
            Some(note) => format!("Submitting live order… · {note}"),
            None => "Submitting live order to the CLOB…".into(),
        };
        self.modal = None;
    }

    fn set_modal_error(&mut self, e: String) {
        if let Some(m) = self.modal.as_mut() {
            m.error = Some(e);
        }
    }

    // --- Reset paper account ----------------------------------------------

    fn reset_modal_key(&mut self, key: KeyEvent) {
        let Some(m) = self.reset_modal.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.reset_modal = None,
            KeyCode::Backspace => {
                m.balance.pop();
            }
            KeyCode::Char('y') | KeyCode::Char('Y') if m.has_copy => m.disable_copy = true,
            KeyCode::Char('n') | KeyCode::Char('N') if m.has_copy => m.disable_copy = false,
            KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => m.balance.push(c),
            KeyCode::Enter => self.submit_reset(),
            _ => {}
        }
    }

    fn update_modal_key(&mut self, key: KeyEvent) {
        let Some(m) = self.update_modal.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.update_modal = None,
            KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l') => {
                m.confirm = !m.confirm;
            }
            KeyCode::Up | KeyCode::Char('k') => m.scroll = m.scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => m.scroll = m.scroll.saturating_add(1),
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.update_modal = None;
                self.start_upgrade();
            }
            KeyCode::Char('n') | KeyCode::Char('N') => self.update_modal = None,
            KeyCode::Enter => {
                let confirm = m.confirm;
                self.update_modal = None;
                if confirm {
                    self.start_upgrade();
                }
            }
            _ => {}
        }
    }

    fn start_upgrade(&mut self) {
        self.status = "Quitting to run upgrade…".into();
        self.should_quit = true;
        self.run_upgrade = true;
    }

    fn submit_reset(&mut self) {
        let (bal_s, disable_copy) = match self.reset_modal.as_ref() {
            Some(m) => (m.balance.clone(), m.disable_copy && m.has_copy),
            None => return,
        };
        let balance = if bal_s.trim().is_empty() {
            default_starting_balance()
        } else {
            match parse_dec(&bal_s) {
                Ok(b) => b,
                Err(e) => {
                    if let Some(m) = self.reset_modal.as_mut() {
                        m.error = Some(e.to_string());
                    }
                    return;
                }
            }
        };
        if balance <= Decimal::ZERO {
            if let Some(m) = self.reset_modal.as_mut() {
                m.error = Some("Balance must be positive.".into());
            }
            return;
        }

        // Wipe the account and start fresh; the engine shares this handle.
        {
            let mut acct = self.account.lock().unwrap();
            *acct = PaperAccount::new(balance, true);
            let _ = store::save_force(&acct); // reset lowers next_id; bypass the stale-write guard
        }
        if disable_copy {
            self.copy_engine.disable_all();
        }
        self.positions_sel = 0;
        self.orders_sel = 0;
        self.history_scroll = 0;
        self.status = if disable_copy {
            format!(
                "Paper account reset — fresh ${} balance; copy-trading disabled.",
                balance.round_dp(2)
            )
        } else {
            format!(
                "Paper account reset — fresh ${} balance, positions and history cleared.",
                balance.round_dp(2)
            )
        };
        self.reset_modal = None;
    }

    // --- Orders ------------------------------------------------------------

    fn cancel_selected_order(&mut self) {
        if self.live {
            self.cancel_selected_live_order();
            return;
        }
        let id = self
            .ordered_paper_orders()
            .get(self.orders_sel)
            .map(|o| o.id);
        let Some(id) = id else { return };
        let mut acct = self.account.lock().unwrap();
        match paper_engine::cancel_order(&mut acct, id) {
            Ok(o) => {
                let _ = store::save(&acct);
                drop(acct);
                self.status = format!(
                    "Cancelled order #{} ({} {} @ {})",
                    o.id, o.side, o.size, o.price
                );
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    /// Cancel the selected live order at the CLOB, in the background. The
    /// order disappears from the list optimistically; the next slow refresh
    /// re-syncs from the CLOB either way.
    fn cancel_selected_live_order(&mut self) {
        let order = self.ordered_live_orders().into_iter().nth(self.orders_sel);
        let Some(order) = order else {
            self.status = "No live order selected.".into();
            return;
        };
        {
            let mut d = self.data.lock().unwrap();
            d.live_orders.retain(|o| o.id != order.id);
        }
        let shared = Arc::clone(&self.data);
        let id = order.id.clone();
        tokio::spawn(async move {
            let msg = match super::live::cancel_order(&id).await {
                Ok(s) => s,
                Err(e) => format!("Cancel FAILED: {e}"),
            };
            shared.lock().unwrap().notices.push(msg);
        });
        self.status = format!(
            "Cancelling live order {}…",
            &order.id[..order.id.len().min(12)]
        );
    }

    // --- Settlement of resolved markets -------------------------------------

    /// Auto-settle (when the setting is on): every held token whose market
    /// has resolved converts to cash — paper settles locally, live redeems
    /// on-chain. Runs each frame; cheap when nothing has resolved.
    fn tick_settlement(&mut self) {
        if !self.settings.auto_settle {
            return;
        }
        let resolutions = {
            let d = self.data.lock().unwrap();
            if d.resolutions.is_empty() {
                return;
            }
            d.resolutions.clone()
        };
        let held: Vec<String> = self
            .account
            .lock()
            .unwrap()
            .positions
            .keys()
            .cloned()
            .collect();
        for token_id in held {
            if let Some(info) = resolutions.get(&token_id) {
                self.settle_token(&token_id, info.clone());
            }
        }
    }

    /// Manual claim (`r` on Positions): redeem the selected position if its
    /// market has resolved.
    fn redeem_selected_position(&mut self) {
        let Some(p) = self.selected_position() else {
            self.status = "No position selected.".into();
            return;
        };
        let info = self
            .data
            .lock()
            .unwrap()
            .resolutions
            .get(&p.token_id)
            .cloned();
        match info {
            Some(info) => self.settle_token(&p.token_id, info),
            None => {
                self.status =
                    "Market not resolved yet — sell early with s, or wait for resolution.".into();
            }
        }
    }

    /// Convert one resolved position to cash: paper settles in the engine,
    /// live submits the on-chain redemption.
    fn settle_token(&mut self, token_id: &str, info: ResolutionInfo) {
        if self.live {
            self.spawn_live_redeem(token_id, &info);
            return;
        }
        let result = {
            let mut acct = self.account.lock().unwrap();
            paper_engine::settle_position(&mut acct, token_id, info.payout, Utc::now())
        };
        match result {
            Ok(t) => {
                let _ = store::save(&self.account.lock().unwrap());
                let _ = crate::guard::clear(token_id);
                let verdict = if info.won { "WON" } else { "LOST" };
                let pnl = t.realized_pnl.unwrap_or_default().round_dp(2);
                self.status = format!(
                    "[paper] {verdict} — settled {} '{}' at ${} (pnl {pnl})",
                    t.size.round_dp(2),
                    t.outcome,
                    info.payout
                );
                let len = self.account.lock().unwrap().positions.len();
                self.positions_sel = self.positions_sel.min(len.saturating_sub(1));
            }
            Err(e) => self.status = format!("Settlement failed: {e}"),
        }
    }

    /// Redeem a resolved live position on-chain (costs Polygon gas). Each
    /// condition is attempted at most once per session; the position itself
    /// disappears on the next wallet hydrate after the transaction lands.
    fn spawn_live_redeem(&mut self, token_id: &str, info: &ResolutionInfo) {
        let Some(condition_id) = info.condition_id.clone() else {
            self.status =
                "Resolved, but no condition ID — redeem with `polymarket ctf redeem`.".into();
            return;
        };
        if !self.attempted_redeems.insert(condition_id.clone()) {
            return;
        }
        let shares = self
            .account
            .lock()
            .unwrap()
            .positions
            .get(token_id)
            .map_or(Decimal::ZERO, |p| p.size);
        let neg_risk = info.neg_risk;
        let outcome_index = info.outcome_index;
        let outcome_count = info.outcome_count;
        let shared = Arc::clone(&self.data);
        self.status = "Redeeming resolved position on-chain…".into();
        tokio::spawn(async move {
            let msg = match crate::commands::ctf::tui_redeem(
                &condition_id,
                neg_risk,
                shares,
                outcome_index,
                outcome_count,
            )
            .await
            {
                Ok(s) => s,
                Err(e) => format!("Redeem failed: {e}"),
            };
            shared.lock().unwrap().notices.push(msg);
        });
    }

    // --- Copy-trading ------------------------------------------------------

    fn copy_modal_key(&mut self, key: KeyEvent) {
        let Some(m) = self.copy_modal.as_mut() else {
            return;
        };
        let fields = m.fields();
        let n = fields.len();
        let field = fields[m.focus];
        match key.code {
            KeyCode::Esc => self.copy_modal = None,
            KeyCode::Enter => self.submit_copy_modal(),
            KeyCode::Up | KeyCode::BackTab => m.focus = m.focus.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab => {
                if m.focus + 1 < n {
                    m.focus += 1;
                }
            }
            // The mirror-sells toggle flips with space or ←→.
            KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right
                if field == CopyField::MirrorSells =>
            {
                m.mirror_sells = !m.mirror_sells;
            }
            // The sizing-mode toggle swaps fixed <-> ratio the same way.
            KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right if field == CopyField::Mode => {
                m.use_ratio = !m.use_ratio;
            }
            KeyCode::Backspace => {
                if let Some(buf) = m.buf(field) {
                    buf.pop();
                }
            }
            KeyCode::Char(c) if !c.is_control() => {
                if let Some(buf) = m.buf(field) {
                    buf.push(c);
                }
            }
            _ => {}
        }
    }

    fn submit_copy_modal(&mut self) {
        let Some(m) = self.copy_modal.as_ref() else {
            return;
        };
        let wallet = m.wallet.trim().to_string();
        if polymarket_client_sdk_v2::types::Address::from_str(&wallet).is_err() {
            self.set_copy_error("Enter a valid 0x wallet address");
            return;
        }
        let nickname = if m.nickname.trim().is_empty() {
            short_wallet(&wallet)
        } else {
            m.nickname.trim().to_string()
        };
        let parse = |s: &str, label: &str| -> Result<Decimal, String> {
            Decimal::from_str(s.trim()).map_err(|_| format!("{label} must be a number"))
        };
        // In ratio mode the sizing field is the multiplier; otherwise it's the
        // fixed dollar size. The unused buffer keeps its default so toggling back
        // and forth loses nothing.
        let sizing_label = if m.use_ratio { "Ratio" } else { "Copy size" };
        let sizing_buf = if m.use_ratio { &m.ratio } else { &m.size };
        let (sizing, max_dollar, min_price, max_price, slippage) = match (
            parse(sizing_buf, sizing_label),
            parse(&m.max_dollar, "Max per trade"),
            parse(&m.min_price, "Min price"),
            parse(&m.max_price, "Max price"),
            parse(&m.slippage, "Slippage"),
        ) {
            (Ok(a), Ok(b), Ok(c), Ok(d), Ok(e)) => (a, b, c, d, e),
            (a, b, c, d, e) => {
                let err = [a.err(), b.err(), c.err(), d.err(), e.err()]
                    .into_iter()
                    .flatten()
                    .next()
                    .unwrap_or_else(|| "Invalid number".into());
                self.set_copy_error(&err);
                return;
            }
        };
        if sizing <= Decimal::ZERO {
            self.set_copy_error(&format!("{sizing_label} must be greater than zero"));
            return;
        }
        if min_price < Decimal::ZERO || max_price > Decimal::ONE || min_price > max_price {
            self.set_copy_error("Price band must satisfy 0 ≤ min ≤ max ≤ 1");
            return;
        }
        // Fixed mode: size is the dollar amount, no ratio. Ratio mode: the leader
        // multiplier drives sizing and copy_size_usd is ignored by the engine.
        let (copy_size_usd, copy_ratio) = if m.use_ratio {
            (Decimal::ZERO, Some(sizing))
        } else {
            (sizing, None)
        };
        let mirror_sells = m.mirror_sells;
        let edit_id = m.edit_id.clone();
        // Editing keeps the follower's id and its enabled/paper state; a new
        // follow gets a fresh id and mirrors in whichever mode this TUI shows.
        let existing = edit_id
            .as_deref()
            .and_then(|id| self.copy_engine.config(id));
        let id = match &edit_id {
            Some(id) => id.clone(),
            None => self.unique_copy_id(&nickname, &wallet),
        };
        let cfg = CopyTrader {
            id: id.clone(),
            wallet,
            nickname: nickname.clone(),
            copy_size_usd,
            copy_ratio,
            max_dollar_cap: max_dollar,
            price_min: min_price,
            price_max: max_price,
            slippage_pct: slippage,
            mirror_sells,
            enabled: existing.as_ref().is_none_or(|c| c.enabled),
            paper: existing.as_ref().map_or(!self.live, |c| c.paper),
        };
        let res = if edit_id.is_some() {
            self.copy_engine.update(&id, cfg)
        } else {
            self.copy_engine.add(cfg)
        };
        match res {
            Ok(()) => {
                if edit_id.is_none() {
                    let _ = self.copy_engine.start(&id);
                    self.status = format!("Following '{nickname}' as '{id}' (running).");
                } else {
                    self.status = format!("Reconfigured '{id}'.");
                }
                self.copy_modal = None;
            }
            Err(e) => self.set_copy_error(&e.to_string()),
        }
    }

    /// Open the follow form pre-filled from the selected follower (edit mode).
    fn open_copy_edit(&mut self) {
        let snap = self.copy_engine.snapshot();
        let Some(s) = snap.get(self.copytrade_sel) else {
            self.status = "No follower selected.".into();
            return;
        };
        match self.copy_engine.config(&s.id) {
            Some(cfg) => self.copy_modal = Some(CopyModal::from_trader(&cfg)),
            None => self.status = "Follower not found.".into(),
        }
    }

    fn set_copy_error(&mut self, e: &str) {
        if let Some(m) = self.copy_modal.as_mut() {
            m.error = Some(e.to_string());
        }
    }

    fn unique_copy_id(&self, nickname: &str, wallet: &str) -> String {
        let base: String = nickname
            .trim()
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect();
        let base = base.trim_matches('-').to_string();
        let base = if base.is_empty() {
            wallet.trim_start_matches("0x").chars().take(8).collect()
        } else {
            base
        };
        let existing: Vec<String> = self
            .copy_engine
            .snapshot()
            .into_iter()
            .map(|s| s.id)
            .collect();
        if !existing.contains(&base) {
            return base;
        }
        (2..)
            .map(|n| format!("{base}-{n}"))
            .find(|cand| !existing.contains(cand))
            .unwrap_or(base)
    }

    fn copytrade_action(&mut self, act: CopyAct) {
        let snap = self.copy_engine.snapshot();
        let Some(s) = snap.get(self.copytrade_sel) else {
            return;
        };
        let id = s.id.clone();
        let res = match act {
            CopyAct::Start => self.copy_engine.start(&id),
            CopyAct::Stop => self.copy_engine.stop(&id),
            CopyAct::Enable => self.copy_engine.set_enabled(&id, true),
            CopyAct::Disable => self.copy_engine.set_enabled(&id, false),
            CopyAct::Delete => self.copy_engine.remove(&id),
        };
        if matches!(act, CopyAct::Delete) {
            let len = self.copy_engine.snapshot().len();
            self.copytrade_sel = self.copytrade_sel.min(len.saturating_sub(1));
        }
        self.status = match res {
            Ok(()) => format!("{} {}", act.verb(), id),
            Err(e) => e.to_string(),
        };
    }
}

// --- Onboarding ----------------------------------------------------------

impl App {
    fn onboarding_key(&mut self, key: KeyEvent) {
        // Collect the key text first to avoid borrow conflicts.
        let key_text = self.onboarding.as_ref().map(|s| s.import_key.clone());
        if key_text.is_none() {
            return;
        }
        match key.code {
            KeyCode::Esc => {
                self.onboarding = None;
                self.view = View::Dashboard;
                self.status = "No wallet configured — browsing markets only. Press Tab/9 for Settings to log in.".to_string();
            }
            KeyCode::Backspace => {
                if let Some(s) = self.onboarding.as_mut() {
                    s.import_key.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(s) = self.onboarding.as_mut() {
                    s.import_key.push(c);
                }
            }
            KeyCode::Enter => {
                self.execute_import_wallet(&key_text.unwrap_or_default());
            }
            _ => {}
        }
    }

    /// Re-read the wallet config from disk into `self.wallet` so the Settings
    /// panel reflects a just-changed proxy override or signature type.
    fn reload_wallet(&mut self) {
        self.wallet = super::live::wallet_info();
    }

    /// Save a new proxy/funder override (or clear it when blank) and refresh the
    /// panel. Returns an error string on bad input.
    fn save_proxy_override(&mut self, input: &str) -> Result<(), String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            config::set_proxy_address(None).map_err(|e| e.to_string())?;
            self.reload_wallet();
            self.status = "Proxy override cleared — using the derived address.".into();
            return Ok(());
        }
        let checksummed = alloy::primitives::Address::from_str(trimmed)
            .map_err(|_| "Invalid address — expected 0x… (40 hex chars).".to_string())?
            .to_string();
        config::set_proxy_address(Some(&checksummed)).map_err(|e| e.to_string())?;
        self.reload_wallet();
        self.status = format!("Proxy set to {checksummed}. Trades now route through it.");
        Ok(())
    }

    /// Cycle eoa → proxy → gnosis-safe and persist, refreshing the panel.
    fn cycle_signature_type(&mut self) {
        let current = config::resolve_signature_type(None).unwrap_or_else(|_| "proxy".into());
        let next = match current.as_str() {
            "eoa" => "proxy",
            "proxy" => "gnosis-safe",
            _ => "eoa",
        };
        match config::set_signature_type(next) {
            Ok(()) => {
                self.reload_wallet();
                self.status = format!("Signature type → {next}.");
            }
            Err(e) => self.status = format!("Failed to set signature type: {e}"),
        }
    }

    fn execute_import_wallet(&mut self, key: &str) {
        let key = key.trim();
        let signer = match LocalSigner::from_str(key) {
            Ok(s) => s.with_chain_id(Some(POLYGON)),
            Err(_) => {
                self.set_onboarding_error(
                    "Invalid private key. Enter a valid hex key.".to_string(),
                );
                return;
            }
        };
        let address = signer.address();
        let key_hex = format!("{:#x}", signer.to_bytes());
        let storage = match config::save_wallet(&key_hex, POLYGON, config::DEFAULT_SIGNATURE_TYPE) {
            Ok(s) => s,
            Err(e) => {
                self.set_onboarding_error(format!("Failed to save wallet: {e}"));
                return;
            }
        };
        let sig_type = config::resolve_signature_type(None)
            .unwrap_or_else(|_| config::DEFAULT_SIGNATURE_TYPE.to_string());
        let proxy = derive_proxy_wallet(address, POLYGON).map(|a| a.to_string());
        self.wallet = Some(WalletInfo {
            eoa: address.to_string(),
            proxy: proxy.clone(),
            trading: if sig_type == "proxy" {
                proxy.unwrap_or_else(|| address.to_string())
            } else {
                address.to_string()
            },
            signature_type: sig_type,
            private_key: Some(key_hex),
            config_path: config::config_path()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        });
        self.live = true;
        self.onboarding = None;
        self.view = View::Dashboard;
        self.status = format!(
            "✓ Wallet imported: {address} ({}).",
            key_storage_label(&storage),
        );
    }

    fn set_onboarding_error(&mut self, msg: String) {
        if let Some(s) = self.onboarding.as_mut() {
            s.error = Some(msg);
        }
    }
}

// --- Logout (Settings tab) -----------------------------------------------

impl App {
    fn logout_modal_key(&mut self, key: KeyEvent) {
        let Some(m) = self.logout_modal.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => {
                self.logout_modal = None;
                self.status = "Logout cancelled.".into();
            }
            KeyCode::Enter => match m.armed_at {
                // First confirmation: arm the timer.
                None => m.armed_at = Some(Instant::now()),
                // Final confirmation: only fires once the timer has elapsed.
                Some(_) if m.remaining_secs() == 0 => self.execute_logout(),
                Some(_) => {} // still counting down; ignore
            },
            _ => {}
        }
    }

    fn execute_logout(&mut self) {
        self.logout_modal = None;
        if let Err(e) = config::delete_config() {
            self.status = format!("Logout failed: {e}");
            return;
        }
        // Drop the in-memory wallet and return to the login screen.
        self.wallet = None;
        self.reveal_key = false;
        self.onboarding = Some(OnboardingState {
            import_key: String::new(),
            error: None,
        });
        self.view = View::Onboarding;
        self.status = "Logged out — key removed from this machine (keychain + config).".into();
    }
}

// --- Wallet action modal (Settings tab) ---------------------------------

/// Whether an import must pause for an overwrite confirmation before writing.
/// Only an existing wallet is at risk; a first-time import has nothing to clobber.
fn import_needs_confirm(config_exists: bool, already_confirmed: bool) -> bool {
    config_exists && !already_confirmed
}

/// Short label for where the key was saved, for the status line.
fn key_storage_label(storage: &config::KeyStorage) -> &'static str {
    match storage {
        config::KeyStorage::Keychain => "key in OS keychain",
        config::KeyStorage::PlaintextFile => "key in config file — plaintext",
    }
}

impl App {
    fn wallet_action_modal_key(&mut self, key: KeyEvent) {
        let Some(m) = self.wallet_action_modal.as_mut() else {
            return;
        };
        match m.action {
            WalletAction::Import => match key.code {
                KeyCode::Esc => {
                    self.wallet_action_modal = None;
                }
                // Editing the key cancels any pending overwrite confirmation.
                KeyCode::Backspace => {
                    m.import_key.pop();
                    m.confirmed = false;
                }
                KeyCode::Char(c) => {
                    m.import_key.push(c);
                    m.confirmed = false;
                }
                KeyCode::Enter => {
                    let key = m.import_key.trim().to_string();
                    if key.is_empty() {
                        m.error = Some("Enter a private key.".to_string());
                        return;
                    }
                    // Validate before touching the config so a bad key can't
                    // close the modal or overwrite the existing wallet.
                    if LocalSigner::from_str(&key).is_err() {
                        m.error = Some("Invalid private key. Enter a valid hex key.".to_string());
                        m.confirmed = false;
                        return;
                    }
                    // Importing replaces the stored key — confirm once first.
                    if import_needs_confirm(config::config_exists(), m.confirmed) {
                        m.confirmed = true;
                        return;
                    }
                    self.execute_import_wallet(&key);
                    self.wallet_action_modal = None;
                }
                _ => {}
            },
            WalletAction::SetProxy => match key.code {
                KeyCode::Esc => {
                    self.wallet_action_modal = None;
                }
                KeyCode::Backspace => {
                    m.import_key.pop();
                }
                KeyCode::Char(c) => {
                    m.import_key.push(c);
                }
                KeyCode::Enter => {
                    let input = m.import_key.clone();
                    match self.save_proxy_override(&input) {
                        Ok(()) => self.wallet_action_modal = None,
                        Err(e) => {
                            if let Some(m) = self.wallet_action_modal.as_mut() {
                                m.error = Some(e);
                            }
                        }
                    }
                }
                _ => {}
            },
        }
    }
}

/// Translate common CLOB errors into actionable advice for the user.
fn friendly_live_order_error(e: anyhow::Error) -> String {
    let msg = e.to_string();
    if msg.contains("maker address not allowed") || msg.contains("deposit wallet flow") {
        let brief = msg.split('{').next().unwrap_or(&msg).trim();
        format!(
            "{brief}. The CLOB doesn't recognize this wallet for trading. Run `polymarket approve set` first, then deposit USDC.e (Polygon) to your proxy wallet from Settings tab."
        )
    } else if msg.contains("insufficient balance") {
        "Live order rejected — not enough USDC.e (buys) or shares (sells) in your wallet. Deposit USDC.e via `polymarket bridge deposit`.".to_string()
    } else if msg.contains("insufficient allowance") {
        "Live order rejected — contract not approved. Run `polymarket approve set` and try again."
            .to_string()
    } else {
        format!("Live order FAILED: {e}")
    }
}

/// `0x1234…cdef` short form for nicknames/listings.
fn short_wallet(wallet: &str) -> String {
    let w = wallet.trim();
    if w.len() <= 12 {
        return w.to_string();
    }
    format!("{}…{}", &w[..6], &w[w.len() - 4..])
}

enum CopyAct {
    Start,
    Stop,
    Enable,
    Disable,
    Delete,
}

impl CopyAct {
    fn verb(&self) -> &'static str {
        match self {
            CopyAct::Start => "Started",
            CopyAct::Stop => "Stopped",
            CopyAct::Enable => "Enabled",
            CopyAct::Disable => "Disabled",
            CopyAct::Delete => "Unfollowed",
        }
    }
}

fn field_mut(m: &mut OrderModal) -> &mut String {
    match m.field {
        ModalField::Amount => &mut m.amount,
        ModalField::Price => &mut m.price,
        ModalField::Size => &mut m.size,
        ModalField::TakeProfit => &mut m.tp,
        ModalField::StopLoss => &mut m.sl,
    }
}

/// The editable fields, in Tab order, for the given order kind and side.
pub(crate) fn modal_fields(kind: OrderKind, side: TradeSide) -> Vec<ModalField> {
    let mut fields = match kind {
        OrderKind::Market => vec![ModalField::Amount],
        OrderKind::Limit => vec![ModalField::Price, ModalField::Size],
        OrderKind::Settlement => Vec::new(), // never an order form kind
    };
    // Take-profit / stop-loss apply to buys only (they exit a new position).
    if side == TradeSide::Buy {
        fields.push(ModalField::TakeProfit);
        fields.push(ModalField::StopLoss);
    }
    fields
}

fn next_field(kind: OrderKind, side: TradeSide, field: ModalField) -> ModalField {
    let fields = modal_fields(kind, side);
    let idx = fields.iter().position(|f| *f == field).unwrap_or(0);
    fields[(idx + 1) % fields.len()]
}

fn parse_dec(s: &str) -> anyhow::Result<Decimal> {
    Decimal::from_str(s.trim()).map_err(|_| anyhow::anyhow!("Enter a number (got '{s}')"))
}

/// Parse an optional percent field: blank → `None`, else `Some(value)` for a
/// positive number (used to arm TP/SL guards).
fn parse_opt_dec(s: &str) -> Option<Decimal> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    Decimal::from_str(t).ok().filter(|v| *v > Decimal::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_queries_pass_through() {
        assert_eq!(normalize_search_query("bitcoin etf"), "bitcoin etf");
    }

    fn paper_app() -> App {
        use rust_decimal_macros::dec;
        let data = std::sync::Arc::new(std::sync::Mutex::new(super::super::data::SharedData::default()));
        let account = std::sync::Arc::new(std::sync::Mutex::new(PaperAccount::new(dec!(1000), false)));
        let copy_engine = crate::copytrade::engine::CopyEngine::new(account.clone(), 15);
        App::new(data, account, copy_engine, false)
    }

    fn press(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn u_opens_update_modal_only_when_update_available() {
        let mut app = paper_app();
        // No update pending: U is a no-op.
        app.update_available = None;
        app.on_key(press('U'));
        assert!(app.update_modal.is_none());

        // Update pending: U opens the confirm modal carrying the tag.
        app.update_available = Some("v9.9.9".into());
        app.on_key(press('U'));
        let m = app.update_modal.as_ref().expect("modal should open");
        assert_eq!(m.tag, "v9.9.9");
        assert!(m.confirm);
        assert!(!app.run_upgrade);
    }

    #[test]
    fn update_modal_yes_starts_upgrade_no_cancels() {
        let mut app = paper_app();
        app.update_available = Some("v9.9.9".into());

        // 'n' dismisses without upgrading.
        app.on_key(press('U'));
        app.on_key(press('n'));
        assert!(app.update_modal.is_none());
        assert!(!app.run_upgrade);

        // 'y' confirms: modal closes, upgrade flagged, TUI quits.
        app.on_key(press('U'));
        app.on_key(press('y'));
        assert!(app.update_modal.is_none());
        assert!(app.run_upgrade);
        assert!(app.should_quit);
    }

    #[test]
    fn copy_form_swaps_sizing_field_without_changing_length() {
        // `focus` indexes into fields(); both modes must stay the same length or
        // the cursor could point past the end after a mode toggle.
        let mut m = CopyModal::default();
        assert!(m.fields().contains(&CopyField::Size));
        assert!(!m.fields().contains(&CopyField::Ratio));
        let fixed_len = m.fields().len();
        m.use_ratio = true;
        assert!(m.fields().contains(&CopyField::Ratio));
        assert!(!m.fields().contains(&CopyField::Size));
        assert_eq!(m.fields().len(), fixed_len);
    }

    #[test]
    fn edit_form_reflects_existing_sizing_mode() {
        use rust_decimal_macros::dec;
        let mut cfg = CopyTrader {
            id: "alice".into(),
            wallet: "0x0000000000000000000000000000000000000001".into(),
            nickname: "alice".into(),
            copy_size_usd: dec!(25),
            copy_ratio: None,
            max_dollar_cap: dec!(100),
            price_min: dec!(0),
            price_max: dec!(1),
            slippage_pct: dec!(2),
            mirror_sells: true,
            enabled: true,
            paper: true,
        };
        // Fixed follower opens in fixed mode with its dollar size.
        let m = CopyModal::from_trader(&cfg);
        assert_eq!(m.edit_id.as_deref(), Some("alice"));
        assert!(!m.use_ratio);
        assert_eq!(m.size, "25");
        // A ratio follower opens in ratio mode showing the multiplier.
        cfg.copy_ratio = Some(dec!(0.1));
        let m = CopyModal::from_trader(&cfg);
        assert!(m.use_ratio);
        assert_eq!(m.ratio, "0.1");
    }

    #[test]
    fn paper_orders_sort_by_price_both_directions() {
        use rust_decimal_macros::dec;
        let mk = |id: u64, price| OpenOrder {
            id,
            created_at: Utc::now(),
            token_id: id.to_string(),
            question: "Q".into(),
            outcome: "Yes".into(),
            side: TradeSide::Buy,
            price,
            size: dec!(1),
        };
        let mut orders = vec![mk(1, dec!(0.30)), mk(2, dec!(0.10)), mk(3, dec!(0.20))];
        // Column 4 = Price. Descending (asc=false) is the sort default.
        sort_paper_orders(&mut orders, 4, false);
        assert_eq!(
            orders.iter().map(|o| o.id).collect::<Vec<_>>(),
            vec![1, 3, 2]
        );
        sort_paper_orders(&mut orders, 4, true);
        assert_eq!(
            orders.iter().map(|o| o.id).collect::<Vec<_>>(),
            vec![2, 3, 1]
        );
    }

    #[test]
    fn event_urls_reduce_to_slug() {
        assert_eq!(
            normalize_search_query("https://polymarket.com/event/will-x-happen?tid=99"),
            "will x happen"
        );
    }

    #[test]
    fn nested_market_urls_take_last_segment() {
        assert_eq!(
            normalize_search_query("https://polymarket.com/event/fed-rates/fed-cut-in-june"),
            "fed cut in june"
        );
    }

    #[test]
    fn modal_fields_include_tp_sl_on_buys_only() {
        let buy = modal_fields(OrderKind::Market, TradeSide::Buy);
        assert!(buy.contains(&ModalField::TakeProfit));
        assert!(buy.contains(&ModalField::StopLoss));
        let sell = modal_fields(OrderKind::Market, TradeSide::Sell);
        assert_eq!(sell, vec![ModalField::Amount]);
    }

    #[test]
    fn tab_cycles_through_buy_fields() {
        let f = ModalField::Amount;
        let f = next_field(OrderKind::Market, TradeSide::Buy, f);
        assert_eq!(f, ModalField::TakeProfit);
        let f = next_field(OrderKind::Market, TradeSide::Buy, f);
        assert_eq!(f, ModalField::StopLoss);
        let f = next_field(OrderKind::Market, TradeSide::Buy, f);
        assert_eq!(f, ModalField::Amount);
    }

    #[test]
    fn opt_dec_parses_blank_and_values() {
        use rust_decimal_macros::dec;
        assert_eq!(parse_opt_dec(""), None);
        assert_eq!(parse_opt_dec("  "), None);
        assert_eq!(parse_opt_dec("25"), Some(dec!(25)));
        assert_eq!(parse_opt_dec("-5"), None);
    }

    #[test]
    fn logout_timer_gates_final_confirm() {
        // Unarmed: shows the full delay, final confirm not yet possible.
        let unarmed = LogoutModal { armed_at: None };
        assert_eq!(unarmed.remaining_secs(), LOGOUT_DELAY_SECS);
        // Just armed: still counting down (> 0 means Enter won't log out yet).
        let fresh = LogoutModal {
            armed_at: Some(Instant::now()),
        };
        assert!(fresh.remaining_secs() > 0);
        // Armed in the past: timer elapsed, final confirm unlocked (0 remaining).
        if let Some(past) =
            Instant::now().checked_sub(std::time::Duration::from_secs(LOGOUT_DELAY_SECS + 1))
        {
            let elapsed = LogoutModal {
                armed_at: Some(past),
            };
            assert_eq!(elapsed.remaining_secs(), 0);
        }
    }

    #[test]
    fn import_confirms_before_overwriting_existing_wallet() {
        // Existing wallet, not yet confirmed → pause and ask.
        assert!(import_needs_confirm(true, false));
        // Existing wallet, user confirmed → proceed with overwrite.
        assert!(!import_needs_confirm(true, true));
        // No wallet yet → nothing to clobber, import straight away.
        assert!(!import_needs_confirm(false, false));
    }
}
