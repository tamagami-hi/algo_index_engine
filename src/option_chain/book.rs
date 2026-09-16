use std::collections::HashMap;

use serde::Serialize;

use super::metrics::{ChainMetrics, StraddleRow, market_atm_index, metrics, straddle_rows};
use super::table::{OptionTable, build_tables, feed_key};
use crate::dhan_api::feed::{Message, Packet};
use crate::dhan_api::instruments::{
    Catalog, ExchangeSegment, InstrumentMaster, OptionType, SpotInstrument, UnderlyingKey,
};

type FeedKey = (ExchangeSegment, i32);

#[derive(Clone, Copy, Debug)]
struct Leg {
    chain: usize,
    side: OptionType,
    row: usize,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct StrikeRow {
    pub(crate) strike: f64,
    pub(crate) call: Option<Quote>,
    pub(crate) put: Option<Quote>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct Quote {
    pub(crate) ltp: f64,
    pub(crate) bid: f64,
    pub(crate) bid_quantity: f64,
    pub(crate) ask: f64,
    pub(crate) ask_quantity: f64,
    pub(crate) oi: f64,
    pub(crate) change_in_oi: f64,
    pub(crate) volume: f64,
    pub(crate) change: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct SideColumns {
    pub(crate) ltp: Vec<f64>,
    pub(crate) bid: Vec<f64>,
    pub(crate) bid_quantity: Vec<f64>,
    pub(crate) ask: Vec<f64>,
    pub(crate) ask_quantity: Vec<f64>,
    pub(crate) oi: Vec<f64>,
    pub(crate) change_in_oi: Vec<f64>,
    pub(crate) volume: Vec<f64>,
    pub(crate) change: Vec<f64>,
    pub(crate) quoted: Vec<bool>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ChainColumns {
    #[serde(flatten)]
    pub(crate) metrics: ChainMetrics,
    pub(crate) market_atm_row: Option<usize>,
    pub(crate) strike: Vec<f64>,
    pub(crate) call: SideColumns,
    pub(crate) put: SideColumns,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ChainView {
    #[serde(flatten)]
    pub(crate) metrics: ChainMetrics,
    pub(crate) straddles: Vec<StraddleRow>,
    pub(crate) rows: Vec<StrikeRow>,
}

pub(crate) struct ChainBook {
    tables: Vec<OptionTable>,
    legs: HashMap<FeedKey, Vec<Leg>>,
    spots: HashMap<FeedKey, Vec<usize>>,
    references: HashMap<FeedKey, String>,
    reference_prices: HashMap<String, f64>,
    applied: u64,
    unmatched: u64,
}

impl ChainBook {
    pub(crate) fn build(master: &InstrumentMaster, catalog: &Catalog) -> Self {
        let chains: Vec<(UnderlyingKey, String, SpotInstrument)> = catalog
            .underlyings
            .iter()
            .map(|underlying| {
                (
                    underlying.key.clone(),
                    underlying.expiry.clone(),
                    underlying.spot.clone(),
                )
            })
            .collect();

        let tables = build_tables(master, &chains);

        let mut legs: HashMap<FeedKey, Vec<Leg>> = HashMap::new();
        let mut spots: HashMap<FeedKey, Vec<usize>> = HashMap::new();

        for (chain, table) in tables.iter().enumerate() {
            for row in 0..table.len() {
                if let Some(security_id) = table.calls.security_id[row].as_deref()
                    && let Some(key) = feed_key(table.underlying.segment, security_id)
                {
                    legs.entry(key).or_default().push(Leg {
                        chain,
                        side: OptionType::Call,
                        row,
                    });
                }
                if let Some(security_id) = table.puts.security_id[row].as_deref()
                    && let Some(key) = feed_key(table.underlying.segment, security_id)
                {
                    legs.entry(key).or_default().push(Leg {
                        chain,
                        side: OptionType::Put,
                        row,
                    });
                }
            }
            if let Some(key) = feed_key(table.spot.segment, &table.spot.security_id) {
                spots.entry(key).or_default().push(chain);
            }
        }

        let references = catalog
            .spot_labels
            .iter()
            .filter_map(|(subscription, label)| {
                feed_key(subscription.segment, &subscription.security_id)
                    .map(|key| (key, label.clone()))
            })
            .collect();

        Self {
            tables,
            legs,
            spots,
            references,
            reference_prices: HashMap::new(),
            applied: 0,
            unmatched: 0,
        }
    }

    pub(crate) fn chains(&self) -> usize {
        self.tables.len()
    }

    pub(crate) fn stats(&self) -> (u64, u64, std::collections::BTreeMap<String, f64>) {
        (
            self.applied,
            self.unmatched,
            self.reference_prices
                .iter()
                .map(|(label, price)| (label.clone(), *price))
                .collect(),
        )
    }

    pub(crate) fn metrics(&self) -> Vec<ChainMetrics> {
        self.tables.iter().map(metrics).collect()
    }

    pub(crate) fn view(&self, symbol: &str) -> Option<ChainView> {
        let table = self
            .tables
            .iter()
            .find(|table| table.underlying.symbol.eq_ignore_ascii_case(symbol))?;

        let straddles = market_atm_index(table)
            .map(|row| straddle_rows(table, row))
            .unwrap_or_default();

        let rows = (0..table.len())
            .map(|row| StrikeRow {
                strike: table.strikes[row],
                call: quote_at(&table.calls, row),
                put: quote_at(&table.puts, row),
            })
            .collect();

        Some(ChainView {
            metrics: metrics(table),
            straddles,
            rows,
        })
    }

    pub(crate) fn columns(&self, symbol: &str) -> Option<ChainColumns> {
        let table = self
            .tables
            .iter()
            .find(|table| table.underlying.symbol.eq_ignore_ascii_case(symbol))?;

        Some(ChainColumns {
            metrics: metrics(table),
            market_atm_row: market_atm_index(table),
            strike: table.strikes.clone(),
            call: columns_of(&table.calls),
            put: columns_of(&table.puts),
        })
    }


    pub(crate) fn symbols(&self) -> Vec<String> {
        self.tables
            .iter()
            .map(|table| table.underlying.symbol.clone())
            .collect()
    }

    pub(crate) fn apply(&mut self, message: &Message) {
        let Some(segment) = message.header.segment else {
            self.unmatched += 1;
            return;
        };
        let key = (segment, message.header.security_id);

        if let Some(label) = self.references.get(&key).cloned()
            && let Some(price) = reference_price_of(&message.packet)
            && price > 0.0
        {
            self.reference_prices.insert(label, price);
        }

        if let Some(chains) = self.spots.get(&key) {
            if let Some(price) = reference_price_of(&message.packet)
                && price > 0.0
            {
                for chain in chains.clone() {
                    self.tables[chain].spot_price = price;
                    self.tables[chain].spot_updates += 1;
                }
                self.applied += 1;
            }
            return;
        }

        let Some(legs) = self.legs.get(&key).cloned() else {
            self.unmatched += 1;
            return;
        };

        for leg in legs {
            let table = &mut self.tables[leg.chain];
            let block = match leg.side {
                OptionType::Call => &mut table.calls,
                OptionType::Put => &mut table.puts,
            };
            if leg.row >= block.ltp.len() {
                continue;
            }
            apply_packet(block, leg.row, &message.packet);
            self.applied += 1;
        }
    }
}

fn columns_of(block: &super::table::Block) -> SideColumns {
    SideColumns {
        ltp: block.ltp.clone(),
        bid: block.bid.clone(),
        bid_quantity: block.bid_quantity.clone(),
        ask: block.ask.clone(),
        ask_quantity: block.ask_quantity.clone(),
        oi: block.oi.clone(),
        change_in_oi: block.change_in_oi.clone(),
        volume: block.volume.clone(),
        change: block.change.clone(),
        quoted: (0..block.ltp.len()).map(|row| block.is_quoted(row)).collect(),
    }
}

fn quote_at(block: &super::table::Block, row: usize) -> Option<Quote> {
    block.security_id[row].as_ref().map(|_| Quote {
        ltp: block.ltp[row],
        bid: block.bid[row],
        bid_quantity: block.bid_quantity[row],
        ask: block.ask[row],
        ask_quantity: block.ask_quantity[row],
        oi: block.oi[row],
        change_in_oi: block.change_in_oi[row],
        volume: block.volume[row],
        change: block.change[row],
    })
}

fn reference_price_of(packet: &Packet) -> Option<f64> {
    match packet {
        Packet::Index { last_price } | Packet::Ticker { last_price, .. } => {
            Some(f64::from(*last_price))
        }
        Packet::Quote { last_price, .. } => Some(f64::from(*last_price)),
        Packet::Full(full) => Some(f64::from(full.last_price)),
        _ => None,
    }
}

fn apply_packet(block: &mut super::table::Block, row: usize, packet: &Packet) {
    match packet {
        Packet::Full(full) => {
            block.ltp[row] = f64::from(full.last_price);
            block.volume[row] = f64::from(full.volume);
            block.oi[row] = f64::from(full.open_interest);
            block.oi_day_high[row] = f64::from(full.open_interest_day_high);
            block.oi_day_low[row] = f64::from(full.open_interest_day_low);
            block.total_buy_quantity[row] = f64::from(full.total_buy_quantity);
            block.total_sell_quantity[row] = f64::from(full.total_sell_quantity);
            block.average_price[row] = f64::from(full.average_price);
            block.day_open[row] = f64::from(full.day_open);
            block.day_high[row] = f64::from(full.day_high);
            block.day_low[row] = f64::from(full.day_low);
            block.day_close[row] = f64::from(full.day_close);
            block.last_trade_time[row] = i64::from(full.last_trade_time);
            if let Some((price, quantity)) = full.best_bid() {
                block.bid[row] = f64::from(price);
                block.bid_quantity[row] = f64::from(quantity);
            }
            if let Some((price, quantity)) = full.best_ask() {
                block.ask[row] = f64::from(price);
                block.ask_quantity[row] = f64::from(quantity);
            }
            refresh_changes(block, row);
            block.updates[row] += 1;
        }
        Packet::Ticker {
            last_price,
            last_trade_time,
        } => {
            block.ltp[row] = f64::from(*last_price);
            block.last_trade_time[row] = i64::from(*last_trade_time);
            refresh_changes(block, row);
            block.updates[row] += 1;
        }
        Packet::Quote { last_price, volume } => {
            block.ltp[row] = f64::from(*last_price);
            block.volume[row] = f64::from(*volume);
            refresh_changes(block, row);
            block.updates[row] += 1;
        }
        Packet::OpenInterest { open_interest } => {
            block.oi[row] = f64::from(*open_interest);
            refresh_changes(block, row);
        }
        Packet::PrevClose {
            close,
            open_interest,
        } => {
            block.previous_close[row] = f64::from(*close);
            block.previous_oi[row] = f64::from(*open_interest);
            refresh_changes(block, row);
        }
        Packet::Index { .. } | Packet::Disconnect { .. } | Packet::Unknown { .. } => {}
    }
}

fn refresh_changes(block: &mut super::table::Block, row: usize) {
    if block.previous_close[row] > 0.0 {
        block.change[row] = block.ltp[row] - block.previous_close[row];
    }
    if block.previous_oi[row] > 0.0 {
        block.change_in_oi[row] = block.oi[row] - block.previous_oi[row];
    }
}

#[cfg(test)]
#[path = "../../tests/option_chain/book.rs"]
mod tests;
