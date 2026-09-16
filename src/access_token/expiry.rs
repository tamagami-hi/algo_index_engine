use anyhow::Result;
use serde::{Deserialize, Serialize};

pub(crate) const TOKEN_VALIDITY_SECONDS: i64 = 24 * 60 * 60;

const IST_OFFSET_SECONDS: i64 = 5 * 3600 + 1800;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExpirySource {
    Stated,
    JwtClaim,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Expiry {
    pub(crate) at_unix_seconds: i64,
    pub(crate) source: ExpirySource,
}

impl Expiry {
    pub(crate) const fn is_expired(&self, now_unix_seconds: i64) -> bool {
        now_unix_seconds >= self.at_unix_seconds
    }

    pub(crate) const fn remaining_seconds(&self, now_unix_seconds: i64) -> i64 {
        self.at_unix_seconds - now_unix_seconds
    }
}

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

pub(crate) fn parse_stated_expiry(value: &StatedExpiry) -> Option<i64> {
    match value {
        StatedExpiry::Number(number) => normalise_epoch(*number),
        StatedExpiry::Text(text) => {
            let text = text.trim();
            if text.is_empty() {
                return None;
            }
            if let Ok(number) = text.parse::<f64>() {
                return normalise_epoch(number);
            }
            parse_ist_timestamp(text)
        }
        StatedExpiry::Null => None,
    }
}

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

fn parse_ist_timestamp(text: &str) -> Option<i64> {
    let (date, time) = text.split_once(['T', ' '])?;

    let mut date_parts = date.split('-');
    let year = date_parts.next()?.parse::<i64>().ok()?;
    let month = date_parts.next()?.parse::<i64>().ok()?;
    let day = date_parts.next()?.parse::<i64>().ok()?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

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

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_position = (month + 9) % 12;
    let day_of_year = (153 * month_position + 2) / 5 + day - 1;
    let day_of_era =
        year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

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
            b'=' => break,
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

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(untagged)]
pub(crate) enum StatedExpiry {
    Number(f64),
    Text(String),
    #[default]
    Null,
}

pub(crate) fn now_unix_seconds() -> Result<i64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| anyhow::anyhow!("system clock is set before the Unix epoch"))?
        .as_secs() as i64)
}

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

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let shifted = days_since_epoch + 719_468;
    let era = if shifted >= 0 {
        shifted / 146_097
    } else {
        (shifted - 146_096) / 146_097
    };
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_position = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_position + 2) / 5 + 1;
    let month = if month_position < 10 {
        month_position + 3
    } else {
        month_position - 9
    };

    (if month <= 2 { year + 1 } else { year }, month, day)
}

pub(crate) fn humanize(seconds: i64) -> String {
    let seconds = seconds.abs();
    match seconds {
        0..=119 => format!("{seconds}s"),
        120..=7199 => format!("{}m", seconds / 60),
        7200..=172_799 => format!("{}h{:02}m", seconds / 3600, (seconds % 3600) / 60),
        _ => format!("{}d", seconds / 86_400),
    }
}
