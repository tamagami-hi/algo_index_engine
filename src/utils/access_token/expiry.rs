//! When a Dhan access token stops working.
//!
//! Dhan states the rule plainly in the v2 authentication docs: a token is issued
//! "for a validity of 24 hours", and every endpoint that mints one returns an
//! `expiryTime` described as "set to 24 hours from generation".
//!
//! Three sources of truth, strongest first, because each is available in different
//! circumstances:
//!
//! 1. **A stated expiry** from the token route — `expires_at` from the Cal Spread
//!    route, or `expiryTime` when pointed straight at a Dhan endpoint. Exact.
//! 2. **The token's own `exp` claim.** Dhan access tokens are JWTs, so the token
//!    carries its expiry inside it. This needs no extra field and no network call,
//!    which makes it the fallback that almost always works.
//! 3. **24 hours from when we fetched it**, the documented default, used only when
//!    neither of the above is available.
//!
//! An unknown expiry is never treated as "never expires". It resolves to 24 hours,
//! which is the documented maximum a Dhan token can be good for.

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Documented lifetime of a Dhan access token.
pub(crate) const TOKEN_VALIDITY_SECONDS: i64 = 24 * 60 * 60;

/// IST is a fixed UTC+05:30; Dhan states `expiryTime` in IST.
const IST_OFFSET_SECONDS: i64 = 5 * 3600 + 1800;

/// Where an expiry came from, so logs can say whether it is exact or assumed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExpirySource {
    /// Stated by the token route.
    Stated,
    /// Decoded from the token's own JWT `exp` claim.
    JwtClaim,
    /// Assumed: 24 hours from when the token was fetched.
    Assumed,
}

impl ExpirySource {
    pub(crate) const fn describe(self) -> &'static str {
        match self {
            Self::Stated => "stated by the token route",
            Self::JwtClaim => "from the token's JWT exp claim",
            Self::Assumed => "assumed 24h, neither the route nor the token said",
        }
    }
}

/// An access token's expiry, as Unix seconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Expiry {
    pub(crate) at_unix_seconds: i64,
    pub(crate) source: ExpirySource,
}

impl Expiry {
    /// Whether the token is past its expiry at `now`.
    pub(crate) const fn is_expired(&self, now_unix_seconds: i64) -> bool {
        now_unix_seconds >= self.at_unix_seconds
    }

    /// Seconds left before expiry; negative once past it.
    pub(crate) const fn remaining_seconds(&self, now_unix_seconds: i64) -> i64 {
        self.at_unix_seconds - now_unix_seconds
    }
}

/// Resolve a token's expiry from whatever is known about it.
///
/// `stated` is the route's value if it sent one, `fetched_at_unix_seconds` is when we
/// obtained the token.
pub(crate) fn resolve(
    stated: Option<i64>,
    access_token: &str,
    fetched_at_unix_seconds: i64,
) -> Expiry {
    if let Some(at_unix_seconds) = stated {
        return Expiry {
            at_unix_seconds,
            source: ExpirySource::Stated,
        };
    }
    if let Some(at_unix_seconds) = jwt_expiry(access_token) {
        return Expiry {
            at_unix_seconds,
            source: ExpirySource::JwtClaim,
        };
    }
    Expiry {
        at_unix_seconds: fetched_at_unix_seconds + TOKEN_VALIDITY_SECONDS,
        source: ExpirySource::Assumed,
    }
}

/// Normalise a stated expiry into Unix seconds.
///
/// Accepts what the various producers actually emit:
/// - epoch milliseconds (the Cal Spread route's `expires_at`),
/// - epoch seconds,
/// - an ISO timestamp in IST (Dhan's own `expiryTime`, e.g. `2025-09-23T12:37:23`).
///
/// Returns `None` for anything unparseable, which resolves to the 24-hour default
/// rather than to "no expiry".
pub(crate) fn parse_stated_expiry(value: &StatedExpiry) -> Option<i64> {
    match value {
        StatedExpiry::Number(number) => normalise_epoch(*number),
        StatedExpiry::Text(text) => {
            let text = text.trim();
            if text.is_empty() {
                return None;
            }
            // A numeric string is still an epoch.
            if let Ok(number) = text.parse::<f64>() {
                return normalise_epoch(number);
            }
            parse_ist_timestamp(text)
        }
        StatedExpiry::Null => None,
    }
}

/// Epoch seconds and milliseconds are told apart by magnitude.
///
/// Anything at or above 1e11 must be milliseconds: 1e11 seconds is the year 5138,
/// and 1e11 ms is 1973, so the boundary is unambiguous for any real token.
fn normalise_epoch(number: f64) -> Option<i64> {
    if !number.is_finite() || number <= 0.0 {
        return None;
    }
    Some(if number < 1e11 {
        number.round() as i64
    } else {
        (number / 1000.0).round() as i64
    })
}

/// Parse `YYYY-MM-DDTHH:MM:SS` (optionally with fractional seconds) as IST.
///
/// Dhan documents `expiryTime` as "as per IST", so this must not be read as UTC —
/// doing so would put the expiry 5h30m early and throw away a still-valid token.
fn parse_ist_timestamp(text: &str) -> Option<i64> {
    let (date, time) = text.split_once(['T', ' '])?;

    let mut date_parts = date.split('-');
    let year = date_parts.next()?.parse::<i64>().ok()?;
    let month = date_parts.next()?.parse::<i64>().ok()?;
    let day = date_parts.next()?.parse::<i64>().ok()?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    // Drop any fractional seconds or trailing zone marker; Dhan sends neither a zone
    // nor an offset, and the value is IST by definition.
    let time = time.trim_end_matches('Z');
    let time = time.split_once('.').map_or(time, |(head, _)| head);
    let mut time_parts = time.split(':');
    let hour = time_parts.next()?.parse::<i64>().ok()?;
    let minute = time_parts.next()?.parse::<i64>().ok()?;
    let second = time_parts.next().unwrap_or("0").parse::<i64>().ok()?;
    if !(0..=23).contains(&hour) || !(0..=59).contains(&minute) || !(0..=60).contains(&second) {
        return None;
    }

    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - IST_OFFSET_SECONDS)
}

/// A civil date to days since 1970-01-01. Howard Hinnant's `days_from_civil`.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    // Treat March as the first month so the leap day falls at the cycle's end.
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400; // [0, 399]
    let month_position = (month + 9) % 12; // [0, 11]
    let day_of_year = (153 * month_position + 2) / 5 + day - 1; // [0, 365]
    let day_of_era =
        year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year; // [0, 146096]
    era * 146_097 + day_of_era - 719_468
}

/// The `exp` claim of a JWT access token, as Unix seconds.
///
/// Dhan's access token is a JWT (the docs show `eyJ…` and call it "JWT access token"),
/// so its expiry travels with it. Only the payload is read — this is not signature
/// verification, and it is not treated as trusted input beyond reading one number.
pub(crate) fn jwt_expiry(token: &str) -> Option<i64> {
    let payload = token.split('.').nth(1)?;
    let decoded = decode_base64url(payload)?;
    let claims: serde_json::Value = serde_json::from_slice(&decoded).ok()?;

    match claims.get("exp")? {
        serde_json::Value::Number(number) => number.as_f64().and_then(normalise_epoch),
        serde_json::Value::String(text) => text.parse::<f64>().ok().and_then(normalise_epoch),
        _ => None,
    }
}

/// Decode unpadded base64url, as used by JWT segments.
fn decode_base64url(input: &str) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u32;

    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            b'=' => break, // tolerate padding even though JWT omits it
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
        }
    }
    Some(output)
}

/// A stated expiry as it arrives over the wire, before normalisation.
///
/// Untagged because the field is a number from the Cal Spread route, a string from
/// Dhan, and `null` when no session stated one.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum StatedExpiry {
    Number(f64),
    Text(String),
    Null,
}

impl Default for StatedExpiry {
    fn default() -> Self {
        Self::Null
    }
}

/// Current time as Unix seconds.
pub(crate) fn now_unix_seconds() -> Result<i64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| anyhow::anyhow!("system clock is set before the Unix epoch"))?
        .as_secs() as i64)
}

/// Render Unix seconds as an IST timestamp, `YYYY-MM-DDTHH:MM:SS+05:30`.
///
/// Written next to the epoch value in the session file so the expiry is legible
/// without a converter, and in the same timezone Dhan states its own `expiryTime` in.
pub(crate) fn format_ist(unix_seconds: i64) -> String {
    let shifted = unix_seconds + IST_OFFSET_SECONDS;
    let days = shifted.div_euclid(86_400);
    let seconds_of_day = shifted.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);

    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}+05:30",
        seconds_of_day / 3600,
        (seconds_of_day % 3600) / 60,
        seconds_of_day % 60,
    )
}

/// Days since 1970-01-01 to a civil date. Hinnant's `civil_from_days`, the inverse of
/// [`days_from_civil`] above.
///
/// Deliberately a local copy rather than shared with the instrument master's date
/// helper: `utils` should not depend on `dhan_api`, and duplicating twelve lines of a
/// fixed, well-known algorithm is cheaper than inverting that layering.
fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let shifted = days_since_epoch + 719_468;
    let era = if shifted >= 0 {
        shifted / 146_097
    } else {
        (shifted - 146_096) / 146_097
    };
    let day_of_era = shifted - era * 146_097; // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_position = (5 * day_of_year + 2) / 153; // [0, 11], March-based
    let day = day_of_year - (153 * month_position + 2) / 5 + 1;
    let month = if month_position < 10 {
        month_position + 3
    } else {
        month_position - 9
    };

    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// Render a duration in seconds as a compact human string.
pub(crate) fn humanize(seconds: i64) -> String {
    let seconds = seconds.abs();
    match seconds {
        0..=119 => format!("{seconds}s"),
        120..=7199 => format!("{}m", seconds / 60),
        7200..=172_799 => format!("{}h{:02}m", seconds / 3600, (seconds % 3600) / 60),
        _ => format!("{}d", seconds / 86_400),
    }
}
