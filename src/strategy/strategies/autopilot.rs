//! Autopilot: condition-triggered automated entries with built-in TP/SL.
//!
//! This is the PolyGun "AutoPilot" feature. You point it at one or more tokens
//! (the *assets*) and give it a price *range* to watch, a trade *direction*
//! gate, an *amount* per fire, a *window* (cooldown between fires), and
//! optional take-profit / stop-loss exits. Each tick it checks every watched
//! token: when the mid sits inside the range and the direction gate passes, it
//! market-buys `amount` (until the per-token cap is hit), then manages the
//! position it opened with the configured TP/SL.
//!
//! Unlike [`super::tp_sl`] (which only ever exits) autopilot both opens and
//! closes, so it's a complete hands-off loop. Keep amounts small while you
//! learn its behaviour — it will keep firing on every qualifying window.

use std::collections::HashMap;

use polymarket_client_sdk_v2::types::Decimal;
use serde::{Deserialize, Serialize};

use crate::strategy::{Signal, Strategy, StrategyContext};

/// Which way the mid must be moving for an entry to fire.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Direction {
    /// Only buy while the mid is rising over the lookback window.
    Up,
    /// Only buy while the mid is falling over the lookback window.
    Down,
    /// Buy on either direction (no movement gate).
    #[default]
    Both,
}

impl Direction {
    fn label(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::Both => "both",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Params {
    /// Lowest mid (probability, 0..1) that may trigger an entry.
    pub range_low: f64,
    /// Highest mid (probability, 0..1) that may trigger an entry.
    pub range_high: f64,
    /// Direction the mid must be moving for an entry to fire.
    pub direction: Direction,
    /// Ticks back used to judge the direction gate.
    pub lookback: usize,
    /// pUSD to deploy on each entry.
    pub amount: f64,
    /// Stop buying once the position is worth this much pUSD.
    pub max_position_usd: f64,
    /// Minimum ticks between two entries on the same token (the "window").
    pub cooldown_ticks: u64,
    /// Sell once unrealized gain reaches this percent (e.g. 30 = +30%).
    pub take_profit_pct: Option<f64>,
    /// Sell once unrealized loss reaches this percent (e.g. 20 = -20%).
    pub stop_loss_pct: Option<f64>,
    /// Fraction of the held position to sell when an exit triggers (0..1).
    pub sell_fraction: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            range_low: 0.0,
            range_high: 1.0,
            direction: Direction::Up,
            lookback: 5,
            amount: 25.0,
            max_position_usd: 200.0,
            cooldown_ticks: 6,
            take_profit_pct: Some(30.0),
            stop_loss_pct: Some(20.0),
            sell_fraction: 1.0,
        }
    }
}

pub(crate) struct Autopilot {
    params: Params,
    /// Monotonic tick counter, used for the entry cooldown.
    tick: u64,
    /// Tick of the last entry per token (for the cooldown window).
    last_entry: HashMap<String, u64>,
}

impl Autopilot {
    pub fn new(params: Params) -> Self {
        Self {
            params,
            tick: 0,
            last_entry: HashMap::new(),
        }
    }

    /// Whether the cooldown window has elapsed for `token` at the current tick.
    fn cooldown_ready(&self, token: &str) -> bool {
        match self.last_entry.get(token) {
            None => true,
            Some(&last) => self.tick.saturating_sub(last) >= self.params.cooldown_ticks,
        }
    }
}

impl Strategy for Autopilot {
    fn kind(&self) -> &'static str {
        "autopilot"
    }

    fn describe(&self) -> String {
        let mut exits = Vec::new();
        if let Some(tp) = self.params.take_profit_pct {
            exits.push(format!("TP +{tp:.0}%"));
        }
        if let Some(sl) = self.params.stop_loss_pct {
            exits.push(format!("SL -{sl:.0}%"));
        }
        let exit_text = if exits.is_empty() {
            "no auto-exit".to_string()
        } else {
            exits.join(", ")
        };
        format!(
            "Autopilot: buy ${:.0} when mid in {:.2}–{:.2} & moving {} (every {} ticks, ${:.0} cap); {}.",
            self.params.amount,
            self.params.range_low,
            self.params.range_high,
            self.params.direction.label(),
            self.params.cooldown_ticks,
            self.params.max_position_usd,
            exit_text,
        )
    }

    fn on_tick(&mut self, ctx: &StrategyContext) -> Vec<Signal> {
        self.tick += 1;
        let mut signals = Vec::new();

        let amount = dec(self.params.amount);
        let cap = dec(self.params.max_position_usd);
        let low = dec(self.params.range_low);
        let high = dec(self.params.range_high);
        let sell_fraction = dec(self.params.sell_fraction).clamp(Decimal::ZERO, Decimal::ONE);

        for t in &ctx.tokens {
            // --- Exit management first: protect anything we hold. ----------
            if t.position_size > Decimal::ZERO
                && t.avg_price > Decimal::ZERO
                && t.best_bid.is_some()
                && let Some(mid) = t.mid
            {
                let gain_pct = (mid - t.avg_price) / t.avg_price * Decimal::ONE_HUNDRED;
                let hit_tp = self
                    .params
                    .take_profit_pct
                    .is_some_and(|tp| gain_pct >= dec(tp));
                let hit_sl = self
                    .params
                    .stop_loss_pct
                    .is_some_and(|sl| gain_pct <= -dec(sl));
                if hit_tp || hit_sl {
                    let shares = (t.position_size * sell_fraction)
                        .round_dp_with_strategy(2, rust_decimal::RoundingStrategy::ToZero)
                        .min(t.position_size);
                    if shares > Decimal::ZERO {
                        signals.push(Signal::MarketSell {
                            token_id: t.token_id.clone(),
                            shares,
                        });
                        continue; // don't also enter on the same tick
                    }
                }
            }

            // --- Entry: mid in range, direction gate, cooldown, cap, cash. -
            let Some(mid) = t.mid else { continue };
            if mid < low || mid > high {
                continue;
            }
            if !self.direction_ok(t) {
                continue;
            }
            if !self.cooldown_ready(&t.token_id) {
                continue;
            }
            if t.position_value() >= cap || ctx.cash < amount || t.best_ask.is_none() {
                continue;
            }
            signals.push(Signal::MarketBuy {
                token_id: t.token_id.clone(),
                usd: amount,
            });
            self.last_entry.insert(t.token_id.clone(), self.tick);
        }
        signals
    }
}

impl Autopilot {
    /// Whether this tick's mid move satisfies the direction gate.
    fn direction_ok(&self, t: &crate::strategy::TokenView) -> bool {
        match self.params.direction {
            Direction::Both => true,
            dir => {
                let n = t.history.len();
                if n <= self.params.lookback {
                    return false; // not enough history to judge a move
                }
                let now = t.history[n - 1];
                let past = t.history[n - 1 - self.params.lookback];
                match dir {
                    Direction::Up => now > past,
                    Direction::Down => now < past,
                    Direction::Both => true,
                }
            }
        }
    }
}

fn dec(v: f64) -> Decimal {
    Decimal::try_from(v).unwrap_or(Decimal::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::TokenView;
    use rust_decimal_macros::dec;

    fn token(mid: Decimal, history: Vec<Decimal>, pos: Decimal, avg: Decimal) -> TokenView {
        TokenView {
            token_id: "t".into(),
            question: "Q".into(),
            outcome: "Yes".into(),
            best_bid: Some(mid),
            best_ask: Some(mid),
            mid: Some(mid),
            history,
            position_size: pos,
            avg_price: avg,
        }
    }

    fn ctx(t: TokenView) -> StrategyContext {
        StrategyContext {
            cash: dec!(10_000),
            tokens: vec![t],
        }
    }

    #[test]
    fn buys_in_range_when_rising() {
        let mut s = Autopilot::new(Params {
            range_low: 0.30,
            range_high: 0.70,
            direction: Direction::Up,
            lookback: 2,
            cooldown_ticks: 0,
            take_profit_pct: None,
            stop_loss_pct: None,
            ..Default::default()
        });
        let sig = s.on_tick(&ctx(token(
            dec!(0.50),
            vec![dec!(0.40), dec!(0.45), dec!(0.50)],
            dec!(0),
            dec!(0),
        )));
        assert!(matches!(sig.as_slice(), [Signal::MarketBuy { .. }]));
    }

    #[test]
    fn no_buy_outside_range() {
        let mut s = Autopilot::new(Params {
            range_low: 0.30,
            range_high: 0.45,
            direction: Direction::Both,
            cooldown_ticks: 0,
            ..Default::default()
        });
        let sig = s.on_tick(&ctx(token(dec!(0.60), vec![dec!(0.60)], dec!(0), dec!(0))));
        assert!(sig.is_empty());
    }

    #[test]
    fn direction_gate_blocks_wrong_way() {
        let mut s = Autopilot::new(Params {
            direction: Direction::Up,
            lookback: 2,
            cooldown_ticks: 0,
            take_profit_pct: None,
            stop_loss_pct: None,
            ..Default::default()
        });
        // Mid falling — Up gate should block.
        let sig = s.on_tick(&ctx(token(
            dec!(0.50),
            vec![dec!(0.60), dec!(0.55), dec!(0.50)],
            dec!(0),
            dec!(0),
        )));
        assert!(sig.is_empty());
    }

    #[test]
    fn cooldown_blocks_back_to_back_entries() {
        let mut s = Autopilot::new(Params {
            direction: Direction::Both,
            cooldown_ticks: 3,
            take_profit_pct: None,
            stop_loss_pct: None,
            ..Default::default()
        });
        let t = || token(dec!(0.50), vec![dec!(0.50)], dec!(0), dec!(0));
        assert_eq!(s.on_tick(&ctx(t())).len(), 1); // tick 1 fires
        assert!(s.on_tick(&ctx(t())).is_empty()); // tick 2 cooling
        assert!(s.on_tick(&ctx(t())).is_empty()); // tick 3 cooling
        assert_eq!(s.on_tick(&ctx(t())).len(), 1); // tick 4 ready again
    }

    #[test]
    fn respects_position_cap() {
        let mut s = Autopilot::new(Params {
            direction: Direction::Both,
            cooldown_ticks: 0,
            max_position_usd: 50.0,
            take_profit_pct: None,
            stop_loss_pct: None,
            ..Default::default()
        });
        // Already holding 200 shares @ 0.50 mid = $100 > $50 cap.
        let sig = s.on_tick(&ctx(token(
            dec!(0.50),
            vec![dec!(0.50)],
            dec!(200),
            dec!(0.40),
        )));
        assert!(sig.is_empty());
    }

    #[test]
    fn take_profit_exits_held_position() {
        let mut s = Autopilot::new(Params {
            direction: Direction::Both,
            cooldown_ticks: 0,
            take_profit_pct: Some(20.0),
            stop_loss_pct: None,
            ..Default::default()
        });
        // entry 0.50, mark 0.65 → +30% ≥ +20%.
        let sig = s.on_tick(&ctx(token(
            dec!(0.65),
            vec![dec!(0.65)],
            dec!(100),
            dec!(0.50),
        )));
        assert!(matches!(sig.as_slice(), [Signal::MarketSell { .. }]));
    }

    #[test]
    fn stop_loss_exits_held_position() {
        let mut s = Autopilot::new(Params {
            direction: Direction::Both,
            cooldown_ticks: 0,
            take_profit_pct: None,
            stop_loss_pct: Some(20.0),
            ..Default::default()
        });
        // entry 0.50, mark 0.39 → -22% ≤ -20%.
        let sig = s.on_tick(&ctx(token(
            dec!(0.39),
            vec![dec!(0.39)],
            dec!(100),
            dec!(0.50),
        )));
        assert!(matches!(sig.as_slice(), [Signal::MarketSell { .. }]));
    }
}
