//! Calendar timestamps of public datasets, parsed strictly into epoch milliseconds (UTC).
//!
//! Session dates carry the facts that temporal questions are about, so a date that cannot be
//! parsed is an error, never a silent synthetic fallback.

use crate::LabError;

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
            if leap { 29 } else { 28 }
        }
    }
}

/// Epoch milliseconds of a UTC civil date-time, validating every field.
pub fn civil_utc_ms(
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
) -> Result<u64, String> {
    if !(1970..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
    {
        return Err(format!(
            "invalid date {year:04}-{month:02}-{day:02} {hour:02}:{minute:02}"
        ));
    }
    let days = days_from_civil(year, month, day);
    let secs = days * 86_400 + i64::from(hour) * 3_600 + i64::from(minute) * 60;
    u64::try_from(secs * 1_000).map_err(|_| "date before the epoch".to_owned())
}

/// Parses LoCoMo `session_<n>_date_time`, e.g. `"1:56 pm on 8 May, 2023"` (read as UTC).
pub fn parse_locomo_datetime(raw: &str) -> Result<u64, LabError> {
    let err = |why: &str| LabError::Serialization(format!("locomo date '{raw}': {why}"));
    let s = raw.trim().to_ascii_lowercase();
    let (time_part, date_part) = s.split_once(" on ").ok_or_else(|| err("missing ' on '"))?;
    let (clock, meridiem) = time_part
        .trim()
        .split_once(' ')
        .ok_or_else(|| err("missing am/pm"))?;
    let (h, m) = clock.split_once(':').ok_or_else(|| err("missing ':'"))?;
    let h: u32 = h.parse().map_err(|_| err("bad hour"))?;
    let minute: u32 = m.parse().map_err(|_| err("bad minute"))?;
    if !(1..=12).contains(&h) {
        return Err(err("hour out of 1..=12"));
    }
    let hour = match meridiem.trim() {
        "am" => h % 12,
        "pm" => h % 12 + 12,
        _ => return Err(err("expected am or pm")),
    };
    let date_part = date_part.replace(',', " ");
    let mut parts = date_part.split_whitespace();
    let day: u32 = parts
        .next()
        .and_then(|d| d.parse().ok())
        .ok_or_else(|| err("bad day"))?;
    let month_name = parts.next().ok_or_else(|| err("missing month"))?;
    let month = MONTHS
        .iter()
        .position(|m| *m == month_name)
        .ok_or_else(|| err("unknown month"))?;
    let year: i64 = parts
        .next()
        .and_then(|y| y.parse().ok())
        .ok_or_else(|| err("bad year"))?;
    if parts.next().is_some() {
        return Err(err("trailing text"));
    }
    #[allow(clippy::cast_possible_truncation)]
    civil_utc_ms(year, month as u32 + 1, day, hour, minute).map_err(|e| err(&e))
}

/// Parses LongMemEval `haystack_dates` / `question_date`, e.g. `"2023/05/20 (Sat) 02:21"` (UTC).
pub fn parse_longmemeval_datetime(raw: &str) -> Result<u64, LabError> {
    let err = |why: &str| LabError::Serialization(format!("longmemeval date '{raw}': {why}"));
    let mut parts = raw.split_whitespace();
    let date = parts.next().ok_or_else(|| err("missing date"))?;
    let weekday = parts.next().ok_or_else(|| err("missing weekday"))?;
    let clock = parts.next().ok_or_else(|| err("missing time"))?;
    if parts.next().is_some() || !(weekday.starts_with('(') && weekday.ends_with(')')) {
        return Err(err("unexpected layout"));
    }
    let ymd: Vec<&str> = date.split('/').collect();
    if ymd.len() != 3 {
        return Err(err("expected YYYY/MM/DD"));
    }
    let year: i64 = ymd[0].parse().map_err(|_| err("bad year"))?;
    let month: u32 = ymd[1].parse().map_err(|_| err("bad month"))?;
    let day: u32 = ymd[2].parse().map_err(|_| err("bad day"))?;
    let (h, m) = clock.split_once(':').ok_or_else(|| err("missing ':'"))?;
    let hour: u32 = h.parse().map_err(|_| err("bad hour"))?;
    let minute: u32 = m.parse().map_err(|_| err("bad minute"))?;
    civil_utc_ms(year, month, day, hour, minute).map_err(|e| err(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locomo_dates_parse_to_utc() {
        // 2023-05-08T13:56:00Z
        assert_eq!(
            parse_locomo_datetime("1:56 pm on 8 May, 2023").unwrap(),
            1_683_554_160_000
        );
        // 12:06 am is just after midnight.
        assert_eq!(
            parse_locomo_datetime("12:06 am on 11 November, 2022").unwrap(),
            civil_utc_ms(2022, 11, 11, 0, 6).unwrap()
        );
        assert_eq!(
            parse_locomo_datetime("12:30 pm on 1 January, 2024").unwrap(),
            civil_utc_ms(2024, 1, 1, 12, 30).unwrap()
        );
        assert!(parse_locomo_datetime("13:00 pm on 8 May, 2023").is_err());
        assert!(parse_locomo_datetime("1:56 pm on 31 June, 2023").is_err());
        assert!(parse_locomo_datetime("yesterday").is_err());
    }

    #[test]
    fn longmemeval_dates_parse_to_utc() {
        assert_eq!(
            parse_longmemeval_datetime("2023/05/20 (Sat) 02:21").unwrap(),
            civil_utc_ms(2023, 5, 20, 2, 21).unwrap()
        );
        assert_eq!(civil_utc_ms(2023, 5, 20, 2, 21).unwrap(), 1_684_549_260_000);
        assert!(parse_longmemeval_datetime("2023-05-20 02:21").is_err());
        assert!(parse_longmemeval_datetime("2023/02/29 (Wed) 10:00").is_err());
    }
}
