use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

use super::strike::{Side, StrikeCriteria};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct TimeOfDay {
    minutes: u32,
}

impl TimeOfDay {
    pub(crate) const fn from_minutes(minutes: u32) -> Self {
        Self { minutes }
    }

    pub(crate) const fn minutes(self) -> u32 {
        self.minutes
    }

    fn parse(text: &str) -> Option<Self> {
        let (hours, rest) = text.split_once(':')?;
        let minutes = rest.split_once(':').map_or(rest, |(value, _)| value);
        let hours: u32 = hours.parse().ok()?;
        let minutes: u32 = minutes.parse().ok()?;
        (hours < 24 && minutes < 60).then_some(Self {
            minutes: hours * 60 + minutes,
        })
    }
}

impl std::fmt::Display for TimeOfDay {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:02}:{:02}", self.minutes / 60, self.minutes % 60)
    }
}

impl Serialize for TimeOfDay {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for TimeOfDay {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).ok_or_else(|| D::Error::custom(format!("expected HH:MM, got {text:?}")))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    Sell,
    Buy,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RiskMethod {
    Percent,
    Points,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) struct Threshold {
    pub(crate) method: RiskMethod,
    pub(crate) value: f64,
}

impl Threshold {
    pub(crate) fn points_from(&self, entry_premium: f64) -> f64 {
        match self.method {
            RiskMethod::Percent => entry_premium * self.value / 100.0,
            RiskMethod::Points => self.value,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) struct TrailingRule {
    pub(crate) arm_at: Threshold,
    pub(crate) give_back: Threshold,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) start_after_minutes: Option<u32>,
}

pub(crate) const MAX_DTE: i64 = 6;

/// The days to expiry a strategy is allowed to run on, named one by one. Selecting
/// every day from 0 to `MAX_DTE` is the "all DTE" case; an empty selection is
/// rejected at validation because it could never run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct DteSelection {
    pub(crate) days: std::collections::BTreeSet<i64>,
}

impl Default for DteSelection {
    fn default() -> Self {
        Self::of([0, 1])
    }
}

impl DteSelection {
    pub(crate) fn of(days: impl IntoIterator<Item = i64>) -> Self {
        Self {
            days: days.into_iter().collect(),
        }
    }

    pub(crate) fn all() -> Self {
        Self::of(0..=MAX_DTE)
    }

    pub(crate) fn is_all(&self) -> bool {
        *self == Self::all()
    }

    /// A negative count is an expiry already past and never qualifies even if the
    /// stored selection somehow holds it. An unknown count fails closed.
    pub(crate) fn allows(&self, days_to_expiry: Option<i64>) -> bool {
        days_to_expiry.is_some_and(|days| days >= 0 && self.days.contains(&days))
    }

    pub(crate) fn describe(&self) -> String {
        if self.days.is_empty() {
            "no DTE selected".to_owned()
        } else if self.is_all() {
            format!("all DTE (0-{MAX_DTE})")
        } else {
            let listed: Vec<String> = self.days.iter().map(|day| format!("{day}DTE")).collect();
            listed.join(", ")
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LossCoverage {
    #[default]
    None,
    Breakeven,
    RecoverStoppedLegLoss,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct LegDefinition {
    pub(crate) side: Side,
    pub(crate) action: Action,
    pub(crate) lots: u32,
    pub(crate) strike: StrikeCriteria,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) stop_loss: Option<Threshold>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) target: Option<Threshold>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) trailing: Option<TrailingRule>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct OverallRisk {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) stop_loss: Option<Threshold>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) target: Option<Threshold>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) trailing: Option<TrailingRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) daily_loss_limit: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) daily_profit_target: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum EntryCondition {
    Always,
    ReferenceAbove { reference: String, value: f64 },
    ReferenceAtMost { reference: String, value: f64 },
}

impl EntryCondition {
    pub(crate) fn is_met(&self, reference: impl Fn(&str) -> Option<f64>) -> bool {
        match self {
            Self::Always => true,
            Self::ReferenceAbove { reference: key, value } => {
                reference(key).is_some_and(|observed| observed > *value)
            }
            Self::ReferenceAtMost { reference: key, value } => {
                reference(key).is_some_and(|observed| observed <= *value)
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Strategy {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) notes: String,
    pub(crate) underlying: String,
    pub(crate) entry_time: TimeOfDay,
    pub(crate) exit_time: TimeOfDay,
    #[serde(default)]
    pub(crate) dte: DteSelection,
    #[serde(default = "always")]
    pub(crate) entry_condition: EntryCondition,
    pub(crate) legs: Vec<LegDefinition>,
    #[serde(default)]
    pub(crate) overall: OverallRisk,
    #[serde(default)]
    pub(crate) loss_coverage: LossCoverage,
}

fn always() -> EntryCondition {
    EntryCondition::Always
}

/// How wide the entry window is. One minute: an entry set for 09:16 may only be
/// opened during 09:16, never from 09:17 onward. A restart or a reconnect that
/// lands after the minute has passed skips the day rather than entering late.
pub(crate) const ENTRY_WINDOW_MINUTES: u32 = 1;

impl Strategy {
    pub(crate) fn entry_closes_at(&self) -> TimeOfDay {
        TimeOfDay::from_minutes(self.entry_time.minutes() + ENTRY_WINDOW_MINUTES)
    }

    pub(crate) fn entry_open(&self, now: u32) -> bool {
        now >= self.entry_time.minutes() && now < self.entry_closes_at().minutes()
    }

    /// Whether an already-open position may still be held. Wider than the entry
    /// window: management runs on to the hard exit.
    pub(crate) fn holdable(&self, now: u32) -> bool {
        now >= self.entry_time.minutes() && now < self.exit_time.minutes()
    }
}

impl Strategy {
    pub(crate) fn template(underlying: &str) -> Self {
        let short_leg = |side: Side| LegDefinition {
            side,
            action: Action::Sell,
            lots: 1,
            strike: StrikeCriteria::Relative {
                moneyness: crate::risk_engine::strike::Moneyness::Atm,
                steps: 0,
            },
            stop_loss: Some(Threshold {
                method: RiskMethod::Percent,
                value: 75.0,
            }),
            target: None,
            trailing: None,
        };

        Self {
            id: String::new(),
            name: String::new(),
            notes: String::new(),
            underlying: underlying.to_owned(),
            entry_time: TimeOfDay::from_minutes(9 * 60 + 16),
            exit_time: TimeOfDay::from_minutes(14 * 60 + 59),
            dte: DteSelection::default(),
            entry_condition: EntryCondition::Always,
            legs: vec![short_leg(Side::Call), short_leg(Side::Put)],
            overall: OverallRisk::default(),
            loss_coverage: LossCoverage::Breakeven,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "problem", rename_all = "snake_case")]
pub(crate) enum StrategyError {
    BlankId,
    BlankName,
    IdNotSlug { id: String },
    BlankUnderlying,
    NoLegs,
    NoDteSelected,
    DteOutOfRange { day: i64, max: i64 },
    ExitNotAfterEntry { entry: String, exit: String },
    ZeroLots { leg: usize },
    NegativeThreshold { leg: usize, field: &'static str },
}

impl Strategy {
    pub(crate) fn validate(&self) -> Result<(), StrategyError> {
        if self.id.trim().is_empty() {
            return Err(StrategyError::BlankId);
        }
        if !self
            .id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(StrategyError::IdNotSlug {
                id: self.id.clone(),
            });
        }
        if self.name.trim().is_empty() {
            return Err(StrategyError::BlankName);
        }
        if self.underlying.trim().is_empty() {
            return Err(StrategyError::BlankUnderlying);
        }
        if self.legs.is_empty() {
            return Err(StrategyError::NoLegs);
        }
        if self.dte.days.is_empty() {
            return Err(StrategyError::NoDteSelected);
        }
        if let Some(day) = self
            .dte
            .days
            .iter()
            .copied()
            .find(|day| !(0..=MAX_DTE).contains(day))
        {
            return Err(StrategyError::DteOutOfRange { day, max: MAX_DTE });
        }
        if self.exit_time <= self.entry_time {
            return Err(StrategyError::ExitNotAfterEntry {
                entry: self.entry_time.to_string(),
                exit: self.exit_time.to_string(),
            });
        }
        for (index, leg) in self.legs.iter().enumerate() {
            if leg.lots == 0 {
                return Err(StrategyError::ZeroLots { leg: index });
            }
            if leg.stop_loss.is_some_and(|rule| rule.value <= 0.0) {
                return Err(StrategyError::NegativeThreshold {
                    leg: index,
                    field: "stop_loss",
                });
            }
            if leg.target.is_some_and(|rule| rule.value <= 0.0) {
                return Err(StrategyError::NegativeThreshold {
                    leg: index,
                    field: "target",
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../tests/risk_engine/strategy.rs"]
mod tests;
