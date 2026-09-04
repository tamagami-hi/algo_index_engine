//! Building the Dhan instrument subscription list from the instrument master.
//!
//! Dhan addresses an instrument as `(ExchangeSegment, SecurityId)` and accepts a
//! subscription as JSON over the feed socket:
//!
//! ```json
//! { "RequestCode": 15, "InstrumentCount": 2,
//!   "InstrumentList": [ { "ExchangeSegment": "IDX_I",   "SecurityId": "13" },
//!                       { "ExchangeSegment": "NSE_FNO", "SecurityId": "49081" } ] }
//! ```
//!
//! So the entire job of this module is to turn a 200k-row CSV into the right
//! `InstrumentList`, within Dhan's limits of 100 instruments per message, 5,000 per
//! connection and 5 connections per user.
//!
//! # Order of operations
//!
//! ```no_run
//! # use anyhow::Result;
//! # fn example() -> Result<()> {
//! # use crate::dhan_api::instruments::*;
//! let master = load_instrument_master("data/instruments/dhan_instruments.csv")?;
//! let universe = ChainUniverse::build(&master, &ist_today()?)?;
//!
//! // Before the feed exists: the spot instruments whose prices place ATM.
//! let discovery = discovery_plan(&universe);
//!
//! // ...subscribe `discovery.spot_subscriptions`, collect last prices...
//! # let spot_prices = Default::default();
//! // Then the full plan, with option legs centred on those prices.
//! let plan = build_plan(&universe, &spot_prices, PlanConfig::default())?;
//! # Ok(())
//! # }
//! ```
//!
//! # Why two stages
//!
//! Which strikes are at the money depends on a live price, and a price only arrives
//! over the feed. There is no way to know the option legs before connecting, so the
//! spot list is built first and the legs follow once prices are in. The alternative —
//! subscribing whole chains and filtering client-side — would need roughly 112,000
//! instruments against a 25,000 ceiling.

// Stage two (`build_plan`, `select_window`, `should_recentre`, the feed-packet segment
// codes) has no caller yet: `main` currently stops after the discovery list, and
// `ws_dhan_connection` is wired next. Remove both allows once the feed consumes it.
#![allow(dead_code)]
#![allow(unused_imports)]

pub(crate) mod chain;
pub(crate) mod master;
pub(crate) mod plan;
pub(crate) mod segments;
pub(crate) mod spot;
pub(crate) mod trading_day;

// Re-exported so callers get the whole workflow from `dhan_api::instruments::…`
// without needing to know which submodule each piece lives in.
pub(crate) use chain::{OptionChain, StrikeWindow, select_window, should_recentre};
pub(crate) use master::{
    ChainKind, InstrumentMaster, OptionContract, OptionType, ParseReport, SpotKind, UnderlyingKey,
    from_strike_units, load_instrument_master, to_strike_units,
};
pub(crate) use plan::{
    ChainUniverse, MAX_INSTRUMENTS_PER_ACCOUNT, PlanConfig, SelectedChain, SpotRoute, Subscription,
    SubscriptionPlan, build_plan, discovery_plan,
};
pub(crate) use segments::ExchangeSegment;
pub(crate) use spot::SpotInstrument;
pub(crate) use trading_day::ist_today;
