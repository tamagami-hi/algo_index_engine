use std::sync::LazyLock;
use std::time::Instant;

use serde::Serialize;

static ORIGIN: LazyLock<Instant> = LazyLock::new(Instant::now);

pub(crate) fn monotonic_millis() -> u64 {
    ORIGIN.elapsed().as_millis() as u64
}

pub(crate) fn age_since(stamp: u64) -> u64 {
    monotonic_millis().saturating_sub(stamp)
}

pub(crate) const DEFAULT_UNDERLYING_MAX_AGE_MS: u64 = 5_000;
pub(crate) const DEFAULT_OPTION_MAX_AGE_MS: u64 = 10_000;
pub(crate) const DEFAULT_FEED_SILENCE_MAX_MS: u64 = 5_000;

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct Freshness {
    pub(crate) underlying_max_age_ms: u64,
    pub(crate) option_max_age_ms: u64,
    pub(crate) feed_silence_max_ms: u64,
}

fn env_millis(variable: &str, fallback: u64) -> u64 {
    std::env::var(variable)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(fallback)
}

static FRESHNESS: LazyLock<Freshness> = LazyLock::new(|| Freshness {
    underlying_max_age_ms: env_millis(
        "BLACKBOX_UNDERLYING_MAX_AGE_MS",
        DEFAULT_UNDERLYING_MAX_AGE_MS,
    ),
    option_max_age_ms: env_millis("BLACKBOX_OPTION_MAX_AGE_MS", DEFAULT_OPTION_MAX_AGE_MS),
    feed_silence_max_ms: env_millis("BLACKBOX_FEED_SILENCE_MAX_MS", DEFAULT_FEED_SILENCE_MAX_MS),
});

pub(crate) fn freshness() -> Freshness {
    *FRESHNESS
}

pub(crate) fn usable_price(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QuoteSide {
    Bid,
    Ask,
}

impl QuoteSide {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Bid => "bid",
            Self::Ask => "ask",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) struct UsableQuote {
    pub(crate) premium: f64,
    pub(crate) bid: Option<f64>,
    pub(crate) ask: Option<f64>,
    pub(crate) age_ms: u64,
}

impl UsableQuote {
    pub(crate) fn side(&self, side: QuoteSide) -> Option<f64> {
        match side {
            QuoteSide::Bid => self.bid,
            QuoteSide::Ask => self.ask,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "problem", rename_all = "snake_case")]
pub(crate) enum QuoteProblem {
    NeverQuoted,
    Stale { age_ms: u64, limit_ms: u64 },
    PremiumUnavailable,
    NoUsableSide { side: QuoteSide },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RawQuote {
    pub(crate) quoted: bool,
    pub(crate) received_at: u64,
    pub(crate) premium: f64,
    pub(crate) bid: f64,
    pub(crate) ask: f64,
}

pub(crate) fn inspect_quote(
    raw: RawQuote,
    needed: Option<QuoteSide>,
) -> Result<UsableQuote, QuoteProblem> {
    inspect_quote_at(
        raw,
        needed,
        monotonic_millis(),
        freshness().option_max_age_ms,
    )
}

pub(crate) fn inspect_quote_at(
    raw: RawQuote,
    needed: Option<QuoteSide>,
    now_ms: u64,
    limit_ms: u64,
) -> Result<UsableQuote, QuoteProblem> {
    if !raw.quoted {
        return Err(QuoteProblem::NeverQuoted);
    }

    let age_ms = now_ms.saturating_sub(raw.received_at);
    if age_ms > limit_ms {
        return Err(QuoteProblem::Stale { age_ms, limit_ms });
    }

    let Some(premium) = usable_price(raw.premium) else {
        return Err(QuoteProblem::PremiumUnavailable);
    };

    let quote = UsableQuote {
        premium,
        bid: usable_price(raw.bid),
        ask: usable_price(raw.ask),
        age_ms,
    };

    if let Some(side) = needed
        && quote.side(side).is_none()
    {
        return Err(QuoteProblem::NoUsableSide { side });
    }

    Ok(quote)
}

#[cfg(test)]
#[path = "../../tests/option_chain/quality.rs"]
mod tests;
