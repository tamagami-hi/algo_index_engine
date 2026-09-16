use std::ops::RangeInclusive;

use serde::Serialize;

use super::table::OptionTable;
use crate::dhan_api::instruments::{from_strike_units, to_strike_units};

pub(crate) const MARKET_ATM_WINDOW: usize = 5;
pub(crate) const MAX_PAIN_WINDOW: usize = 10;
pub(crate) const STRADDLE_WINDOW: usize = 10;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct StraddleRow {
    pub(crate) strike: f64,
    pub(crate) straddle_price: f64,
    pub(crate) change: f64,
    pub(crate) call_ltp: f64,
    pub(crate) put_ltp: f64,
    pub(crate) combined_oi: f64,
    pub(crate) combined_oi_change: f64,
    pub(crate) combined_volume: f64,
    pub(crate) imbalance: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ChainMetrics {
    pub(crate) segment: String,
    pub(crate) symbol: String,
    pub(crate) expiry: String,
    pub(crate) lot_size: u32,
    pub(crate) strikes: usize,
    pub(crate) quoted_strikes: usize,
    pub(crate) strike_step: f64,
    pub(crate) spot_price: f64,
    pub(crate) spot_atm: Option<f64>,
    pub(crate) market_atm: Option<f64>,
    pub(crate) max_pain: Option<f64>,
    pub(crate) atm_straddle: Option<f64>,
    pub(crate) synthetic_future: Option<f64>,
    pub(crate) atm_imbalance: Option<f64>,
    pub(crate) total_call_oi: f64,
    pub(crate) total_put_oi: f64,
    pub(crate) total_combined_oi: f64,
    pub(crate) call_volume: f64,
    pub(crate) put_volume: f64,
    pub(crate) pcr_oi: f64,
    pub(crate) pcr_volume: f64,
}

pub(crate) fn clamp_window(
    center: usize,
    left: usize,
    right: usize,
    len: usize,
) -> RangeInclusive<usize> {
    if len == 0 {
        return RangeInclusive::new(1, 0);
    }
    center.saturating_sub(left)..=(center + right).min(len - 1)
}

fn safe_sum(values: &[f64]) -> f64 {
    values.iter().filter(|value| value.is_finite()).sum()
}

fn is_valid_price(value: f64) -> bool {
    value > 0.0 && value.is_finite()
}

pub(crate) fn spot_atm_units(spot_units: i64, step_units: i64) -> i64 {
    if step_units <= 0 || spot_units <= 0 {
        return spot_units;
    }
    let base = spot_units.div_euclid(step_units) * step_units;
    let remainder = spot_units - base;
    if remainder * 2 < step_units {
        base
    } else {
        base + step_units
    }
}

pub(crate) fn spot_atm(table: &OptionTable) -> Option<f64> {
    if !is_valid_price(table.spot_price) || table.strikes.is_empty() {
        return None;
    }
    let units = spot_atm_units(to_strike_units(table.spot_price), table.strike_step_units);
    Some(from_strike_units(units))
}

pub(crate) fn spot_atm_index(table: &OptionTable) -> Option<usize> {
    let atm = spot_atm(table)?;
    table.find_nearest_strike_index(to_strike_units(atm))
}

pub(crate) fn market_atm_index(table: &OptionTable) -> Option<usize> {
    let anchor = spot_atm_index(table)?;
    let window = clamp_window(anchor, MARKET_ATM_WINDOW, MARKET_ATM_WINDOW, table.len());

    let mut best = None;
    let mut cheapest = f64::MAX;
    for row in window {
        let call = table.calls.ltp[row];
        let put = table.puts.ltp[row];
        if is_valid_price(call) && is_valid_price(put) {
            let straddle = call + put;
            if straddle < cheapest {
                cheapest = straddle;
                best = Some(row);
            }
        }
    }
    Some(best.unwrap_or(anchor))
}

pub(crate) fn max_pain_index(table: &OptionTable, market_atm_row: usize) -> Option<usize> {
    if table.strikes.is_empty() {
        return None;
    }
    let window = clamp_window(market_atm_row, MAX_PAIN_WINDOW, MAX_PAIN_WINDOW, table.len());

    let mut best = market_atm_row;
    let mut least_pain = f64::MAX;
    for candidate in window.clone() {
        let settle = table.strikes[candidate];
        let mut pain = 0.0;
        for row in window.clone() {
            let strike = table.strikes[row];
            if settle > strike {
                pain += (settle - strike) * table.calls.oi[row].max(0.0);
            } else if strike > settle {
                pain += (strike - settle) * table.puts.oi[row].max(0.0);
            }
        }
        if pain < least_pain {
            least_pain = pain;
            best = candidate;
        }
    }
    Some(best)
}

pub(crate) fn imbalance_at(table: &OptionTable, row: usize) -> Option<f64> {
    let bids = table.calls.bid_quantity.get(row).copied().unwrap_or(0.0)
        + table.puts.bid_quantity.get(row).copied().unwrap_or(0.0);
    let asks = table.calls.ask_quantity.get(row).copied().unwrap_or(0.0)
        + table.puts.ask_quantity.get(row).copied().unwrap_or(0.0);
    let total = bids + asks;
    (total > 0.0).then(|| (bids - asks) / total)
}

pub(crate) fn straddle_rows(table: &OptionTable, market_atm_row: usize) -> Vec<StraddleRow> {
    clamp_window(
        market_atm_row,
        STRADDLE_WINDOW,
        STRADDLE_WINDOW,
        table.len(),
    )
    .map(|row| StraddleRow {
        strike: table.strikes[row],
        straddle_price: table.calls.ltp[row] + table.puts.ltp[row],
        change: table.calls.change[row] + table.puts.change[row],
        call_ltp: table.calls.ltp[row],
        put_ltp: table.puts.ltp[row],
        combined_oi: table.calls.oi[row] + table.puts.oi[row],
        combined_oi_change: table.calls.change_in_oi[row] + table.puts.change_in_oi[row],
        combined_volume: table.calls.volume[row] + table.puts.volume[row],
        imbalance: imbalance_at(table, row),
    })
    .collect()
}

pub(crate) fn metrics(table: &OptionTable) -> ChainMetrics {
    let total_call_oi = safe_sum(&table.calls.oi);
    let total_put_oi = safe_sum(&table.puts.oi);
    let call_volume = safe_sum(&table.calls.volume);
    let put_volume = safe_sum(&table.puts.volume);

    let market_atm_row = market_atm_index(table);
    let max_pain_row = market_atm_row.and_then(|row| max_pain_index(table, row));

    let quoted_strikes = (0..table.len())
        .filter(|row| table.calls.is_quoted(*row) || table.puts.is_quoted(*row))
        .count();

    ChainMetrics {
        segment: table.underlying.segment.as_str().to_owned(),
        symbol: table.underlying.symbol.clone(),
        expiry: table.expiry.clone(),
        lot_size: table.lot_size,
        strikes: table.len(),
        quoted_strikes,
        strike_step: from_strike_units(table.strike_step_units),
        spot_price: table.spot_price,
        spot_atm: spot_atm(table),
        market_atm: market_atm_row.map(|row| table.strikes[row]),
        max_pain: max_pain_row.map(|row| table.strikes[row]),
        atm_straddle: market_atm_row.map(|row| table.calls.ltp[row] + table.puts.ltp[row]),
        synthetic_future: market_atm_row
            .map(|row| table.strikes[row] + table.calls.ltp[row] - table.puts.ltp[row]),
        atm_imbalance: market_atm_row.and_then(|row| imbalance_at(table, row)),
        total_call_oi,
        total_put_oi,
        total_combined_oi: total_call_oi + total_put_oi,
        call_volume,
        put_volume,
        pcr_oi: if total_call_oi > 0.0 {
            total_put_oi / total_call_oi
        } else {
            0.0
        },
        pcr_volume: if call_volume > 0.0 {
            put_volume / call_volume
        } else {
            0.0
        },
    }
}

#[cfg(test)]
#[path = "../../tests/option_chain/metrics.rs"]
mod tests;
