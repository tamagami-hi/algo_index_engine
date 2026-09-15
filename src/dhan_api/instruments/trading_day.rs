use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

const IST_OFFSET_SECONDS: i64 = 5 * 3600 + 1800;
const SECONDS_PER_DAY: i64 = 86_400;

pub(crate) fn ist_today() -> Result<String> {
    let unix_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is set before the Unix epoch")?
        .as_secs() as i64;

    Ok(format_iso_date(unix_seconds + IST_OFFSET_SECONDS))
}

fn format_iso_date(shifted_seconds: i64) -> String {
    let days = shifted_seconds.div_euclid(SECONDS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
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

pub(crate) fn parse_iso_date(value: &str) -> Result<(i64, i64, i64)> {
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        anyhow::bail!("expected YYYY-MM-DD, received {value:?}");
    }

    let number = |from: usize, to: usize| -> Result<i64> {
        value[from..to]
            .parse::<i64>()
            .with_context(|| format!("expected YYYY-MM-DD, received {value:?}"))
    };
    let year = number(0, 4)?;
    let month = number(5, 7)?;
    let day = number(8, 10)?;

    if !(1..=12).contains(&month) || !(1..=days_in_month(year, month)).contains(&day) {
        anyhow::bail!("expected a real calendar date, received {value:?}");
    }
    Ok((year, month, day))
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}
