use std::collections::BTreeMap;

use crate::dhan_api::instruments::{
    ExchangeSegment, InstrumentMaster, OptionType, SpotInstrument, UnderlyingKey, from_strike_units,
};

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Block {
    pub(crate) security_id: Vec<Option<String>>,
    pub(crate) ltp: Vec<f64>,
    pub(crate) volume: Vec<f64>,
    pub(crate) oi: Vec<f64>,
    pub(crate) oi_day_high: Vec<f64>,
    pub(crate) oi_day_low: Vec<f64>,
    pub(crate) bid: Vec<f64>,
    pub(crate) bid_quantity: Vec<f64>,
    pub(crate) ask: Vec<f64>,
    pub(crate) ask_quantity: Vec<f64>,
    pub(crate) total_buy_quantity: Vec<f64>,
    pub(crate) total_sell_quantity: Vec<f64>,
    pub(crate) average_price: Vec<f64>,
    pub(crate) day_open: Vec<f64>,
    pub(crate) day_high: Vec<f64>,
    pub(crate) day_low: Vec<f64>,
    pub(crate) day_close: Vec<f64>,
    pub(crate) previous_close: Vec<f64>,
    pub(crate) previous_oi: Vec<f64>,
    pub(crate) change: Vec<f64>,
    pub(crate) change_in_oi: Vec<f64>,
    pub(crate) last_trade_time: Vec<i64>,
    pub(crate) received_at: Vec<u64>,
    pub(crate) updates: Vec<u64>,
}

impl Block {
    pub(crate) fn zeroed(size: usize) -> Self {
        Self {
            security_id: vec![None; size],
            ltp: vec![0.0; size],
            volume: vec![0.0; size],
            oi: vec![0.0; size],
            oi_day_high: vec![0.0; size],
            oi_day_low: vec![0.0; size],
            bid: vec![0.0; size],
            bid_quantity: vec![0.0; size],
            ask: vec![0.0; size],
            ask_quantity: vec![0.0; size],
            total_buy_quantity: vec![0.0; size],
            total_sell_quantity: vec![0.0; size],
            average_price: vec![0.0; size],
            day_open: vec![0.0; size],
            day_high: vec![0.0; size],
            day_low: vec![0.0; size],
            day_close: vec![0.0; size],
            previous_close: vec![0.0; size],
            previous_oi: vec![0.0; size],
            change: vec![0.0; size],
            change_in_oi: vec![0.0; size],
            last_trade_time: vec![0; size],
            received_at: vec![0; size],
            updates: vec![0; size],
        }
    }

    pub(crate) fn is_quoted(&self, row: usize) -> bool {
        self.updates.get(row).is_some_and(|count| *count > 0)
    }

    pub(crate) fn raw_quote(&self, row: usize) -> super::quality::RawQuote {
        super::quality::RawQuote {
            quoted: self.is_quoted(row),
            received_at: self.received_at.get(row).copied().unwrap_or(0),
            premium: self.ltp.get(row).copied().unwrap_or(0.0),
            bid: self.bid.get(row).copied().unwrap_or(0.0),
            ask: self.ask.get(row).copied().unwrap_or(0.0),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct OptionTable {
    pub(crate) underlying: UnderlyingKey,
    pub(crate) expiry: String,
    pub(crate) lot_size: u32,
    pub(crate) strike_step_units: i64,
    pub(crate) strike_units: Vec<i64>,
    pub(crate) strikes: Vec<f64>,
    pub(crate) calls: Block,
    pub(crate) puts: Block,
    pub(crate) spot: SpotInstrument,
    pub(crate) spot_price: f64,
    pub(crate) spot_received_at: u64,
    pub(crate) spot_updates: u64,
}

impl OptionTable {
    pub(crate) fn len(&self) -> usize {
        self.strikes.len()
    }

    pub(crate) fn find_nearest_strike_index(&self, strike_units: i64) -> Option<usize> {
        if self.strike_units.is_empty() {
            return None;
        }
        match self.strike_units.binary_search(&strike_units) {
            Ok(index) => Some(index),
            Err(0) => Some(0),
            Err(index) if index >= self.strike_units.len() => Some(self.strike_units.len() - 1),
            Err(index) => {
                let left = self.strike_units[index - 1];
                let right = self.strike_units[index];
                if strike_units - left <= right - strike_units {
                    Some(index - 1)
                } else {
                    Some(index)
                }
            }
        }
    }
}

pub(crate) fn strike_step_units(strike_units: &[i64]) -> i64 {
    if strike_units.len() < 2 {
        return 0;
    }
    let mut gaps: Vec<i64> = strike_units
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect();
    gaps.sort_unstable();
    gaps[gaps.len() / 2]
}

pub(crate) fn build_tables(
    master: &InstrumentMaster,
    chains: &[(UnderlyingKey, String, SpotInstrument)],
) -> Vec<OptionTable> {
    let mut tables = Vec::with_capacity(chains.len());

    for (key, expiry, spot) in chains {
        let mut rows: BTreeMap<i64, (Option<&str>, Option<&str>)> = BTreeMap::new();
        let mut lot_size = 0;

        for contract in &master.options {
            if contract.segment != key.segment
                || contract.underlying_symbol != key.symbol
                || contract.expiry != *expiry
            {
                continue;
            }
            let entry = rows.entry(contract.strike_units).or_insert((None, None));
            match contract.option_type {
                OptionType::Call => entry.0 = Some(contract.security_id.as_str()),
                OptionType::Put => entry.1 = Some(contract.security_id.as_str()),
            }
            if lot_size == 0 && contract.lot_size > 0 {
                lot_size = contract.lot_size;
            }
        }

        if rows.is_empty() {
            continue;
        }

        let size = rows.len();
        let mut table = OptionTable {
            underlying: key.clone(),
            expiry: expiry.clone(),
            lot_size,
            strike_step_units: 0,
            strike_units: Vec::with_capacity(size),
            strikes: Vec::with_capacity(size),
            calls: Block::zeroed(size),
            puts: Block::zeroed(size),
            spot: spot.clone(),
            spot_price: 0.0,
            spot_received_at: 0,
            spot_updates: 0,
        };

        for (row, (units, (call, put))) in rows.into_iter().enumerate() {
            table.strike_units.push(units);
            table.strikes.push(from_strike_units(units));
            table.calls.security_id[row] = call.map(str::to_owned);
            table.puts.security_id[row] = put.map(str::to_owned);
        }
        table.strike_step_units = strike_step_units(&table.strike_units);
        tables.push(table);
    }

    tables
}

pub(crate) fn feed_key(
    segment: ExchangeSegment,
    security_id: &str,
) -> Option<(ExchangeSegment, i32)> {
    security_id.parse::<i32>().ok().map(|id| (segment, id))
}

#[cfg(test)]
#[path = "../../tests/option_chain/table.rs"]
mod tests;
